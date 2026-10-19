//! Claim tracking and the filing assistant (MDR §5.5). OpenClaim prepares
//! local packages and hands off to the official site; it never submits.

use super::{IdParam, PageParams, Pager, bump_version, parse_params};
use crate::ctx::Ctx;
use crate::modules::people::ensure_person;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require, scalar};
use openclaim_domain::claims::{ClaimStatus, TransitionEvidence, check_transition};
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Fields the owner may choose to prefill. Sensitive fields are not offered.
pub const PREFILL_FIELDS: &[&str] = &["name", "email", "phone", "mailing_address"];

fn load(ctx: &Ctx<'_>, id: &str) -> OcResult<Value> {
    let mut row = require(
        query_one(ctx.conn, "SELECT c.*, o.title AS opportunity_title, o.official_url, o.claim_url, o.claim_deadline, o.deadline_verified, o.status AS opportunity_status, o.verification_state, o.claim_method, o.proof_required, p.display_name FROM claims c JOIN opportunities o ON o.id = c.opportunity_id JOIN people p ON p.id = c.person_id WHERE c.id = ?1", [id])?,
        "claim",
        id,
    )?;
    let notes = row["notes_enc"].as_str().and_then(|n| ctx.svc.vault().decrypt_str(n).ok());
    if let Some(o) = row.as_object_mut() {
        o.remove("notes_enc");
        o.insert("notes".into(), if ctx.actor.is_owner() { json!(notes) } else { json!(notes.map(|_| "[hidden from agents]")) });
        let status = ClaimStatus::parse(o["status"].as_str().unwrap_or("")).unwrap_or(ClaimStatus::Discovered);
        o.insert("allowed_next".into(), json!(status.allowed_next().iter().map(|s| s.as_str()).collect::<Vec<_>>()));
    }
    Ok(row)
}

fn record_event(ctx: &Ctx<'_>, claim_id: &str, event_type: &str, from: Option<&str>, to: Option<&str>, evidence_id: Option<&str>, meta: Value) -> OcResult<()> {
    exec(
        ctx.conn,
        "INSERT INTO claim_events(id, claim_id, event_type, from_status, to_status, who_recorded, timestamp, evidence_id, metadata) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        (openclaim_db::new_id(), claim_id, event_type, from, to, ctx.actor_id(), &ctx.now, evidence_id, openclaim_security::redact::redact_json(&meta).to_string()),
    )?;
    Ok(())
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        person_id: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(flatten)]
        page: PageParams,
    }
    let p: P = parse_params(p)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let rows = query_all(
        ctx.conn,
        &format!("SELECT c.id, c.person_id, p.display_name, c.opportunity_id, o.title AS opportunity_title, c.status, o.claim_deadline, o.deadline_verified, o.status AS opportunity_status, c.external_confirmation_id, c.manual_paid_amount, c.currency, c.updated_at, c.version
                  FROM claims c JOIN opportunities o ON o.id = c.opportunity_id JOIN people p ON p.id = c.person_id
                  WHERE c.created_at <= ?1 AND (?2 IS NULL OR c.person_id = ?2) AND (?3 IS NULL OR c.status = ?3) ORDER BY c.updated_at DESC{}", pager.sql_limit()),
        (&pager.snapshot, &p.person_id, &p.status),
    )?;
    Ok(pager.wrap(rows, None))
}

fn op_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let claim = load(ctx, &p.id)?;
    let events = query_all(ctx.conn, "SELECT event_type, from_status, to_status, who_recorded, timestamp, evidence_id, metadata FROM claim_events WHERE claim_id = ?1 ORDER BY timestamp", [&p.id])?;
    let docs = query_all(ctx.conn, "SELECT d.id, d.doc_type, l.validated, l.locator FROM document_links l JOIN documents d ON d.id = l.document_id WHERE l.target_kind = 'claim' AND l.target_id = ?1", [&p.id])?;
    let packages = query_all(ctx.conn, "SELECT id, allowed_domain, created_at, expires_at FROM claim_packages WHERE claim_id = ?1 ORDER BY created_at DESC", [&p.id])?;
    Ok(json!({"claim": claim, "events": events, "documents": docs, "packages": packages}))
}

#[derive(Deserialize, JsonSchema)]
pub struct InitParams {
    pub person_id: String,
    pub opportunity_id: String,
}

fn op_init(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: InitParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let opp = require(query_one(ctx.conn, "SELECT id, title, verification_state, status FROM opportunities WHERE id = ?1", [&p.opportunity_id])?, "opportunity", &p.opportunity_id)?;
    if let Some(existing) = scalar::<String, _>(ctx.conn, "SELECT id FROM claims WHERE person_id = ?1 AND opportunity_id = ?2", (&p.person_id, &p.opportunity_id))? {
        return Ok(json!({"id": existing, "existing": true}));
    }
    if !crate::modules::people::consent_ok(ctx.conn, &p.person_id)? {
        ctx.warn("this person has not recorded explicit authorization (people.consent); record it before filing anything on their behalf");
    }
    if opp["verification_state"] != "verified" {
        ctx.warn(format!("opportunity is {}: verify the official terms before relying on it", opp["verification_state"]));
    }
    let id = openclaim_db::new_id();
    exec(ctx.conn, "INSERT INTO claims(id, person_id, opportunity_id, status, created_at, updated_at) VALUES (?1,?2,?3,'discovered',?4,?4)", (&id, &p.person_id, &p.opportunity_id, &ctx.now))?;
    record_event(ctx, &id, "created", None, Some("discovered"), None, json!({}))?;
    crate::modules::alerts::schedule_deadline_alerts(ctx.conn, "claim", &p.opportunity_id, &ctx.now)?;
    Ok(json!({"id": id, "status": "discovered", "existing": false}))
}

pub fn checklist(ctx: &Ctx<'_>, claim_id: &str) -> OcResult<Value> {
    let claim = load(ctx, claim_id)?;
    let pid = claim["person_id"].as_str().unwrap_or("").to_string();
    let oid = claim["opportunity_id"].as_str().unwrap_or("").to_string();
    let (assessment, _) = crate::modules::matching::evaluate_pair(ctx.conn, &pid, &oid, &ctx.now)?;
    let docs = query_all(ctx.conn, "SELECT d.id, d.doc_type, l.validated FROM document_links l JOIN documents d ON d.id = l.document_id WHERE l.target_kind = 'claim' AND l.target_id = ?1", [claim_id])?;
    let proof_docs = query_all(ctx.conn, "SELECT f.id AS fact_id, f.proof_id FROM consumer_facts f WHERE f.person_id = ?1 AND f.proof_id IS NOT NULL", [&pid])?;
    let mut warnings = Vec::new();
    let today = openclaim_domain::dates::format_date(openclaim_domain::dates::today_utc());
    if let Some(d) = claim["claim_deadline"].as_str() {
        if d < today.as_str() {
            warnings.push(format!("the official deadline {d} has passed"));
        } else if claim["deadline_verified"] != true {
            warnings.push("deadline is not verified against a primary source".into());
        }
    } else {
        warnings.push("no claim deadline is known; check the official notice".into());
    }
    if claim["proof_required"] == "required" && docs.is_empty() && proof_docs.is_empty() {
        warnings.push("the official terms require proof and no document is linked yet".into());
    }
    if claim["claim_method"] == "automatic" {
        warnings.push("this program pays automatically; a claim form may not exist".into());
    }
    if claim["verification_state"] != "verified" {
        warnings.push(format!("opportunity record is {}", claim["verification_state"]));
    }
    let duplicate: i64 = scalar::<i64, _>(ctx.conn, "SELECT count(*) FROM claims WHERE opportunity_id = ?1 AND person_id != ?2 AND status IN ('submitted','pending','approved','paid')", (&oid, &pid))?.unwrap_or(0);
    if duplicate > 0 {
        if let Some((_, rules, _)) = crate::modules::opportunities::current_rules(ctx.conn, &oid)? {
            if rules.one_claim_per.as_deref() == Some("household") {
                warnings.push("another household member already filed; the terms allow one claim per household".into());
            }
        }
    }
    if !crate::modules::people::consent_ok(ctx.conn, &pid)? {
        warnings.push("the claimant has not recorded explicit authorization (people.consent)".into());
    }
    let ready = warnings.is_empty() && assessment.missing_facts.is_empty() && assessment.state == openclaim_domain::rules::MatchState::PotentialMatch;
    Ok(json!({
        "claim_id": claim_id,
        "status": claim["status"],
        "official_url": claim["official_url"],
        "claim_url": claim["claim_url"],
        "deadline": claim["claim_deadline"],
        "deadline_verified": claim["deadline_verified"],
        "eligibility": assessment,
        "linked_documents": docs,
        "missing_facts": assessment.missing_facts,
        "warnings": warnings,
        "ready": ready,
        "next_steps": [
            "Review the eligibility reasons and the original source text",
            "Link receipts or notices as proof (documents.link)",
            "Run claims.prepare to build a local package",
            "Run claims.handoff to open the official site; you complete CAPTCHA, identity checks, signature and Submit",
            "Record the confirmation with claims.record_submission"
        ],
    }))
}

fn op_checklist(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    checklist(ctx, &p.id)
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct PrepareParams {
    pub claim_id: String,
    /// Non-sensitive fields to include for prefill: name, email, phone, mailing_address.
    #[serde(default)]
    pub prefill_fields: Vec<String>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

fn op_prepare(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PrepareParams = parse_params(p)?;
    let check = checklist(ctx, &p.claim_id)?;
    let claim = load(ctx, &p.claim_id)?;
    for f in &p.prefill_fields {
        if !PREFILL_FIELDS.contains(&f.as_str()) {
            return Err(OcError::policy(format!("'{f}' cannot be prefilled; allowed: {}", PREFILL_FIELDS.join(", "))));
        }
    }
    if ctx.actor.is_agent() && !p.prefill_fields.is_empty() {
        return Err(OcError::denied("agents cannot select prefill fields; the owner chooses them"));
    }
    let pid = claim["person_id"].as_str().unwrap_or("").to_string();
    let person = require(query_one(ctx.conn, "SELECT * FROM people WHERE id = ?1", [&pid])?, "person", &pid)?;
    let details = crate::modules::people::details_of(ctx, &person)?.unwrap_or_default();
    let mut prefill = serde_json::Map::new();
    for f in &p.prefill_fields {
        let v = match f.as_str() {
            "name" => Some(person["display_name"].as_str().unwrap_or("").to_string()),
            "email" => details.emails.first().cloned(),
            "phone" => details.phones.first().cloned(),
            "mailing_address" => details.mailing_address.clone(),
            _ => None,
        };
        if let Some(v) = v {
            prefill.insert(f.clone(), json!(v));
        }
    }
    let domain = claim["claim_url"].as_str().or(claim["official_url"].as_str()).and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(str::to_string));
    let package = json!({
        "claim_id": p.claim_id,
        "opportunity": {"id": claim["opportunity_id"], "title": claim["opportunity_title"], "official_url": claim["official_url"], "claim_url": claim["claim_url"], "deadline": claim["claim_deadline"], "deadline_verified": claim["deadline_verified"]},
        "checklist": check,
        "prefill_fields": p.prefill_fields,
        "user_must_complete": ["CAPTCHA", "identity verification", "payment destination", "electronic signature", "Submit"],
        "built_at": ctx.now,
    });
    let package_id = openclaim_db::new_id();
    let expires = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::days(7));
    exec(
        ctx.conn,
        "INSERT INTO claim_packages(id, claim_id, package_json, prefill_enc, allowed_domain, created_by, created_at, expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        (&package_id, &p.claim_id, package.to_string(), if prefill.is_empty() { None } else { Some(ctx.svc.vault().encrypt_str(&Value::Object(prefill.clone()).to_string())) }, &domain, ctx.actor_id(), &ctx.now, &expires),
    )?;
    let from = ClaimStatus::parse(claim["status"].as_str().unwrap_or("")).unwrap_or(ClaimStatus::Discovered);
    let to = if check["ready"] == true { ClaimStatus::Ready } else if !check["missing_facts"].as_array().map(|a| a.is_empty()).unwrap_or(true) || check["warnings"].as_array().is_some_and(|w| w.iter().any(|x| x.as_str().unwrap_or("").contains("proof"))) { ClaimStatus::ProofNeeded } else { ClaimStatus::Reviewing };
    if from != to && from.allowed_next().contains(&to) {
        bump_version(ctx.conn, "claims", &p.claim_id, p.expected_version, &ctx.now)?;
        exec(ctx.conn, "UPDATE claims SET status = ?2 WHERE id = ?1", (&p.claim_id, to.as_str()))?;
        record_event(ctx, &p.claim_id, "prepared", Some(from.as_str()), Some(to.as_str()), None, json!({"package_id": package_id}))?;
    } else {
        record_event(ctx, &p.claim_id, "prepared", None, None, None, json!({"package_id": package_id}))?;
    }
    Ok(json!({"package_id": package_id, "claim_id": p.claim_id, "status": to.as_str(), "ready": check["ready"], "prefill_fields": p.prefill_fields, "allowed_domain": domain, "expires_at": expires, "package": package}))
}

fn op_handoff(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        claim_id: String,
    }
    let p: P = parse_params(p)?;
    let claim = load(ctx, &p.claim_id)?;
    let url = claim["claim_url"].as_str().or(claim["official_url"].as_str()).ok_or_else(|| OcError::not_found("no official URL is known for this opportunity"))?.to_string();
    if !openclaim_security::netpolicy::is_safe_display_link(&url) {
        return Err(OcError::policy("the stored official URL is not a safe https link"));
    }
    let package = query_one(ctx.conn, "SELECT id, allowed_domain, expires_at FROM claim_packages WHERE claim_id = ?1 AND expires_at > ?2 ORDER BY created_at DESC LIMIT 1", (&p.claim_id, &ctx.now))?;
    let from = ClaimStatus::parse(claim["status"].as_str().unwrap_or("")).unwrap_or(ClaimStatus::Discovered);
    if from.allowed_next().contains(&ClaimStatus::OpenedOfficialForm) {
        bump_version(ctx.conn, "claims", &p.claim_id, None, &ctx.now)?;
        exec(ctx.conn, "UPDATE claims SET status = 'opened_official_form' WHERE id = ?1", [&p.claim_id])?;
        record_event(ctx, &p.claim_id, "handoff", Some(from.as_str()), Some("opened_official_form"), None, json!({"url": url}))?;
    }
    Ok(json!({
        "claim_id": p.claim_id,
        "open_url": url,
        "package_id": package.as_ref().map(|p| p["id"].clone()),
        "allowed_domain": package.as_ref().map(|p| p["allowed_domain"].clone()),
        "instructions": [
            "Open the URL in your own browser and confirm it is the official administrator site",
            "If the browser extension is paired, it previews each prefill field before filling; it never clicks Submit",
            "Complete CAPTCHA, identity verification, signature and Submit yourself",
            "Copy the confirmation number and run claims.record_submission"
        ],
        "note": "OpenClaim does not submit claims.",
    }))
}

#[derive(Deserialize, JsonSchema)]
pub struct RecordSubmissionParams {
    pub claim_id: String,
    #[serde(default)]
    pub confirmation: Option<String>,
    #[serde(default)]
    pub document_id: Option<String>,
    /// You confirm the claim was really submitted on the official site.
    pub user_confirmed: bool,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

fn op_record_submission(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RecordSubmissionParams = parse_params(p)?;
    if ctx.actor.is_agent() {
        return Err(OcError::denied("only the claimant records a submission"));
    }
    let claim = load(ctx, &p.claim_id)?;
    let from = ClaimStatus::parse(claim["status"].as_str().unwrap_or("")).unwrap_or(ClaimStatus::Discovered);
    let ev = TransitionEvidence { user_confirmed: p.user_confirmed, confirmation_id: p.confirmation.clone(), document_id: p.document_id.clone(), ..Default::default() };
    let from = if from.allowed_next().contains(&ClaimStatus::Submitted) { from } else if from.allowed_next().contains(&ClaimStatus::OpenedOfficialForm) { ClaimStatus::OpenedOfficialForm } else { from };
    check_transition(from, ClaimStatus::Submitted, &ev)?;
    bump_version(ctx.conn, "claims", &p.claim_id, p.expected_version, &ctx.now)?;
    exec(ctx.conn, "UPDATE claims SET status = 'submitted', submitted_at = ?2, external_confirmation_id = ?3 WHERE id = ?1", (&p.claim_id, &ctx.now, &p.confirmation))?;
    record_event(ctx, &p.claim_id, "submitted", Some(claim["status"].as_str().unwrap_or("")), Some("submitted"), p.document_id.as_deref(), json!({"confirmation": p.confirmation, "user_confirmed": true}))?;
    exec(ctx.conn, "UPDATE alerts SET disposition = 'dismissed' WHERE subject_id = ?1 AND kind = 'deadline' AND disposition = 'pending'", [claim["opportunity_id"].as_str().unwrap_or("")])?;
    Ok(json!({"claim_id": p.claim_id, "status": "submitted", "confirmation": p.confirmation}))
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateParams {
    pub claim_id: String,
    pub status: String,
    #[serde(default)]
    pub user_confirmed: bool,
    #[serde(default)]
    pub confirmation_id: Option<String>,
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Integer minor units for `paid`.
    #[serde(default)]
    pub amount_minor: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

fn op_update(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: UpdateParams = parse_params(p)?;
    let to = ClaimStatus::parse(&p.status).ok_or_else(|| OcError::invalid(format!("unknown status '{}'", p.status)))?;
    if ctx.actor.is_agent() && to.requires_user_confirmation() {
        return Err(OcError::denied("agents cannot record submission, approval, denial or payment"));
    }
    let claim = load(ctx, &p.claim_id)?;
    let from = ClaimStatus::parse(claim["status"].as_str().unwrap_or("")).unwrap_or(ClaimStatus::Discovered);
    let ev = TransitionEvidence { user_confirmed: p.user_confirmed, confirmation_id: p.confirmation_id.clone(), document_id: p.document_id.clone(), note: p.note.clone(), amount_minor: p.amount_minor, currency: p.currency.clone() };
    check_transition(from, to, &ev)?;
    if let Some(d) = &p.document_id {
        require(scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE id = ?1", [d])?, "document", d)?;
        exec(ctx.conn, "INSERT OR REPLACE INTO document_links(document_id, target_kind, target_id, locator, validated, created_at) VALUES (?1,'claim',?2,NULL,1,?3)", (d, &p.claim_id, &ctx.now))?;
    }
    let version = bump_version(ctx.conn, "claims", &p.claim_id, p.expected_version, &ctx.now)?;
    let notes_enc = p.note.as_deref().map(|n| ctx.svc.vault().encrypt_str(n));
    exec(ctx.conn, "UPDATE claims SET status = ?2, notes_enc = COALESCE(?3, notes_enc), external_confirmation_id = COALESCE(?4, external_confirmation_id), submitted_at = CASE WHEN ?2 = 'submitted' THEN ?5 ELSE submitted_at END WHERE id = ?1", (&p.claim_id, to.as_str(), notes_enc, &p.confirmation_id, &ctx.now))?;
    if to == ClaimStatus::Paid {
        exec(ctx.conn, "UPDATE claims SET manual_paid_amount = ?2, currency = ?3 WHERE id = ?1", (&p.claim_id, p.amount_minor, p.currency.as_deref().map(str::to_uppercase)))?;
        exec(ctx.conn, "INSERT INTO ledger(id, claim_id, amount_minor, currency, received_on, note, recorded_by, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", (openclaim_db::new_id(), &p.claim_id, p.amount_minor.unwrap_or(0), p.currency.as_deref().map(str::to_uppercase).unwrap_or_default(), &ctx.now[..10], &p.note, ctx.actor_id(), &ctx.now))?;
    }
    record_event(ctx, &p.claim_id, "status", Some(from.as_str()), Some(to.as_str()), p.document_id.as_deref(), json!({"user_confirmed": p.user_confirmed, "confirmation": p.confirmation_id, "amount_minor": p.amount_minor}))?;
    if to.is_terminal() || to.requires_user_confirmation() {
        exec(ctx.conn, "UPDATE alerts SET disposition = 'dismissed' WHERE subject_id = ?1 AND kind = 'deadline' AND disposition = 'pending'", [claim["opportunity_id"].as_str().unwrap_or("")])?;
    }
    Ok(json!({"claim_id": p.claim_id, "status": to.as_str(), "version": version}))
}

fn op_history(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        claim_id: String,
    }
    let p: P = parse_params(p)?;
    load(ctx, &p.claim_id)?;
    Ok(json!({"claim_id": p.claim_id, "events": query_all(ctx.conn, "SELECT event_type, from_status, to_status, who_recorded, timestamp, evidence_id, metadata FROM claim_events WHERE claim_id = ?1 ORDER BY timestamp", [&p.claim_id])?}))
}

fn op_export(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        person_id: Option<String>,
        #[serde(default)]
        format: Option<String>,
    }
    let p: P = parse_params(p)?;
    let list = op_list(ctx, json!({"person_id": p.person_id, "limit": 500}))?;
    let items = list["items"].as_array().cloned().unwrap_or_default();
    let cols = ["id", "display_name", "opportunity_title", "status", "claim_deadline", "external_confirmation_id", "manual_paid_amount", "currency", "updated_at"];
    Ok(match p.format.as_deref().unwrap_or("json") {
        "csv" => json!({"format": "csv", "content": crate::modules::opportunities::to_csv(&items, &cols), "count": items.len()}),
        "markdown" => json!({"format": "markdown", "content": crate::modules::opportunities::to_markdown(&items, &cols), "count": items.len()}),
        _ => json!({"format": "json", "items": items, "count": items.len()}),
    })
}

fn op_ledger(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let rows = query_all(ctx.conn, "SELECT l.id, l.claim_id, o.title, l.amount_minor, l.currency, l.received_on, l.note, l.recorded_by FROM ledger l LEFT JOIN claims c ON c.id = l.claim_id LEFT JOIN opportunities o ON o.id = c.opportunity_id ORDER BY l.received_on DESC", [])?;
    let totals = query_all(ctx.conn, "SELECT currency, sum(amount_minor) AS total_minor, count(*) AS n FROM ledger GROUP BY currency", [])?;
    Ok(json!({"items": rows, "totals": totals, "note": "amounts are integer minor units; no commissions are taken"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct PackageParams {
    pub package_id: String,
    /// The page origin the extension is on (https://host).
    #[serde(default)]
    pub origin: Option<String>,
}

/// Returns the prefill values to the owner or the paired extension, only for the allowed domain.
fn op_prefill_package(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PackageParams = parse_params(p)?;
    let pkg = require(query_one(ctx.conn, "SELECT * FROM claim_packages WHERE id = ?1", [&p.package_id])?, "package", &p.package_id)?;
    if pkg["expires_at"].as_str().unwrap_or("") < ctx.now.as_str() {
        return Err(OcError::conflict("this package expired; run claims.prepare again"));
    }
    let is_extension = matches!(&ctx.actor, crate::ctx::Actor::Agent { principal_kind, .. } if principal_kind == "extension");
    if !ctx.actor.is_owner() && !is_extension {
        return Err(OcError::denied("prefill packages are only released to the owner or the paired browser extension"));
    }
    let allowed = pkg["allowed_domain"].as_str().unwrap_or("");
    if let Some(origin) = &p.origin {
        let host = url::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        if !openclaim_security::netpolicy::host_allowed(&host, &[allowed.to_string()]) {
            return Err(OcError::policy(format!("prefill is only allowed on {allowed}, not {host}")));
        }
    } else if is_extension {
        return Err(OcError::invalid("the extension must send the page origin"));
    }
    let prefill: Value = match pkg["prefill_enc"].as_str() {
        Some(enc) => serde_json::from_str(&ctx.svc.vault().decrypt_str(enc).map_err(|e| OcError::internal(e.to_string()))?)?,
        None => json!({}),
    };
    ctx.disclose("claim_package", &p.package_id, "prefill")?;
    Ok(json!({"package_id": p.package_id, "claim_id": pkg["claim_id"], "allowed_domain": allowed, "prefill": prefill, "never_fill": ["password", "ssn", "card", "account", "routing", "signature", "captcha"], "never_submit": true}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("claims.list", "claims.read", Read, "List claim tracking records", PageParams, op_list),
        op!("claims.show", "claims.read", Read, "Claim with events, documents and packages", IdParam, op_show, resource = "id", mcp = "get_claim_status"),
        op!("claims.init", "claims.track", Write, "Start tracking a claim for a person", InitParams, op_init),
        op!("claims.checklist", "claims.read", Read, "Eligibility checklist, warnings and missing proof", IdParam, op_checklist, mcp = "get_claim_checklist"),
        op!("claims.prepare", "claims.draft", Elevated, "Build a local claim package (never submitted externally)", PrepareParams, op_prepare, mcp = "prepare_claim_draft"),
        op!("claims.handoff", "claims.track", Write, "Open the official site: the user completes and submits", super::Empty, op_handoff),
        op!("claims.record_submission", "claims.track", Write, "Record that you submitted on the official site", RecordSubmissionParams, op_record_submission),
        op!("claims.update", "claims.track", Write, "Record a status change with evidence", UpdateParams, op_update),
        op!("claims.history", "claims.read", Read, "Claim event history", super::Empty, op_history),
        op!("claims.export", "claims.read", Read, "Export claims as json, csv or markdown", super::Empty, op_export),
        op!("claims.ledger", "claims.read", Read, "Recovered amounts ledger", super::Empty, op_ledger),
        op!("claims.prefill_package", "extension.prefill", Read, "Release user-approved prefill values for the allowed domain", PackageParams, op_prefill_package),
    ]
}
