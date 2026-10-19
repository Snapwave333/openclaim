//! Local claimant profiles (MDR §5.3). Optional details are encrypted at rest
//! and only shown to agents holding `people.read_sensitive`.

use super::{IdParam, PageParams, Pager, bump_version, opt_date, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require, scalar};
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Optional sensitive details. No SSN or bank fields exist by design.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PersonDetails {
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub mailing_address: Option<String>,
    #[serde(default)]
    pub employers: Vec<String>,
    #[serde(default)]
    pub purchase_categories: Vec<String>,
    #[serde(default)]
    pub memberships: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ConsentFlags {
    /// This adult explicitly authorized handling their submissions.
    #[serde(default)]
    pub authorized_by_adult: bool,
    #[serde(default)]
    pub recorded_at: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

pub fn ensure_person(ctx: &Ctx<'_>, id: &str) -> OcResult<()> {
    require(scalar::<String, _>(ctx.conn, "SELECT id FROM people WHERE id = ?1", [id])?, "person", id).map(|_| ())
}

pub fn details_of(ctx: &Ctx<'_>, row: &Value) -> OcResult<Option<PersonDetails>> {
    let Some(enc) = row["encrypted_optional_details"].as_str() else {
        return Ok(None);
    };
    let json = ctx.svc.vault().decrypt_str(enc).map_err(|e| OcError::internal(e.to_string()))?;
    Ok(Some(serde_json::from_str(&json)?))
}

fn view(ctx: &Ctx<'_>, row: Value) -> OcResult<Value> {
    let id = row["id"].as_str().unwrap_or("").to_string();
    let consent: ConsentFlags = serde_json::from_value(row["consent_flags"].clone()).unwrap_or_default();
    let residences = query_all(
        ctx.conn,
        "SELECT id, jurisdiction, start_date, end_date, provenance FROM residence_periods WHERE person_id = ?1 ORDER BY start_date",
        [&id],
    )?;
    let fact_count: i64 = scalar(ctx.conn, "SELECT count(*) FROM consumer_facts WHERE person_id = ?1", [&id])?.unwrap_or(0);
    let sensitive_ok = ctx.actor.is_owner() || ctx.actor.has_scope("people.read_sensitive", Some(&id));
    let details = if sensitive_ok {
        ctx.disclose("person", &id, "details")?;
        json!(details_of(ctx, &row)?)
    } else if row["encrypted_optional_details"].is_null() {
        Value::Null
    } else {
        json!({"redacted": true, "note": "details require scope people.read_sensitive"})
    };
    Ok(json!({
        "id": id,
        "display_name": row["display_name"],
        "consent": consent,
        "residences": residences,
        "consumer_fact_count": fact_count,
        "details": details,
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
        "version": row["version"],
        "trust": "human_confirmed",
    }))
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PageParams = parse_params(p)?;
    let pager = Pager::from(&p, &ctx.now)?;
    let rows = query_all(
        ctx.conn,
        &format!(
            "SELECT id, display_name, consent_flags_json, created_at, updated_at, version FROM people WHERE created_at <= ?1 ORDER BY display_name{}",
            pager.sql_limit()
        ),
        [&pager.snapshot],
    )?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({"id": r["id"], "display_name": r["display_name"], "consent": r["consent_flags"], "updated_at": r["updated_at"], "version": r["version"]}))
        .collect();
    Ok(pager.wrap(items, None))
}

fn op_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT * FROM people WHERE id = ?1", [&p.id])?, "person", &p.id)?;
    view(ctx, row)
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateParams {
    pub display_name: String,
    #[serde(default)]
    pub details: Option<PersonDetails>,
    #[serde(default)]
    pub consent: Option<ConsentFlags>,
}

fn validate_details(d: &PersonDetails) -> OcResult<()> {
    let all = d
        .aliases
        .iter()
        .chain(&d.emails)
        .chain(&d.phones)
        .chain(&d.employers)
        .chain(&d.purchase_categories)
        .chain(&d.memberships);
    for s in all.chain(d.mailing_address.iter()).chain(d.notes.iter()) {
        if s.len() > 2000 {
            return Err(OcError::invalid("a profile field is longer than 2000 characters"));
        }
        let lower = s.to_lowercase();
        if lower.contains("ssn") || lower.contains("social security") || lower.contains("routing") {
            return Err(OcError::policy(
                "government identifiers and bank details are not stored in profiles; enter them only on the official site when required",
            ));
        }
    }
    Ok(())
}

fn op_create(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: CreateParams = parse_params(p)?;
    if p.display_name.trim().is_empty() {
        return Err(OcError::invalid("display_name is required"));
    }
    let details = p.details.unwrap_or_default();
    validate_details(&details)?;
    let id = openclaim_db::new_id();
    let enc = ctx.svc.vault().encrypt_str(&serde_json::to_string(&details)?);
    let mut consent = p.consent.unwrap_or_default();
    if consent.authorized_by_adult {
        consent.recorded_at = Some(ctx.now.clone());
    }
    exec(
        ctx.conn,
        "INSERT INTO people(id, display_name, encrypted_optional_details, consent_flags_json, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?5)",
        (&id, p.display_name.trim(), enc, serde_json::to_string(&consent)?, &ctx.now),
    )?;
    Ok(json!({"id": id, "display_name": p.display_name.trim(), "version": 1}))
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateParams {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Replaces the stored details entirely.
    #[serde(default)]
    pub details: Option<PersonDetails>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

fn op_update(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: UpdateParams = parse_params(p)?;
    let version = bump_version(ctx.conn, "people", &p.id, p.expected_version, &ctx.now)?;
    if let Some(name) = p.display_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        exec(ctx.conn, "UPDATE people SET display_name = ?2 WHERE id = ?1", (&p.id, name))?;
    }
    if let Some(d) = &p.details {
        validate_details(d)?;
        exec(
            ctx.conn,
            "UPDATE people SET encrypted_optional_details = ?2 WHERE id = ?1",
            (&p.id, ctx.svc.vault().encrypt_str(&serde_json::to_string(d)?)),
        )?;
    }
    exec(ctx.conn, "UPDATE matches SET stale = 1 WHERE person_id = ?1", [&p.id])?;
    Ok(json!({"id": p.id, "version": version}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ResidenceParams {
    pub person_id: String,
    /// State/country code, e.g. CA, NY, US, CA-BC.
    pub jurisdiction: String,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
}

fn op_add_residence(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ResidenceParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let (start, end) = (opt_date(&p.start)?, opt_date(&p.end)?);
    if let (Some(s), Some(e)) = (&start, &end)
        && s > e
    {
        return Err(OcError::invalid("start must not be after end"));
    }
    let j = p.jurisdiction.trim().to_uppercase();
    if j.is_empty() || j.len() > 8 {
        return Err(OcError::invalid("jurisdiction must be a short code such as CA or US"));
    }
    let id = openclaim_db::new_id();
    exec(
        ctx.conn,
        "INSERT INTO residence_periods(id, person_id, jurisdiction, start_date, end_date, provenance, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        (&id, &p.person_id, &j, start, end, format!("entered by {}", ctx.actor_id()), &ctx.now),
    )?;
    bump_version(ctx.conn, "people", &p.person_id, None, &ctx.now)?;
    exec(ctx.conn, "UPDATE matches SET stale = 1 WHERE person_id = ?1", [&p.person_id])?;
    Ok(json!({"id": id, "person_id": p.person_id, "jurisdiction": j}))
}

fn op_remove_residence(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let pid: Option<String> = scalar(ctx.conn, "SELECT person_id FROM residence_periods WHERE id = ?1", [&p.id])?;
    let pid = require(pid, "residence period", &p.id)?;
    exec(ctx.conn, "DELETE FROM residence_periods WHERE id = ?1", [&p.id])?;
    bump_version(ctx.conn, "people", &pid, None, &ctx.now)?;
    exec(ctx.conn, "UPDATE matches SET stale = 1 WHERE person_id = ?1", [&pid])?;
    Ok(json!({"removed": p.id}))
}

fn op_residences(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        person_id: String,
    }
    let p: P = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    Ok(
        json!({"items": query_all(ctx.conn, "SELECT id, jurisdiction, start_date, end_date, provenance, created_at FROM residence_periods WHERE person_id = ?1 ORDER BY start_date", [&p.person_id])?}),
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct ConsentParams {
    pub person_id: String,
    pub authorized_by_adult: bool,
    #[serde(default)]
    pub note: Option<String>,
}

fn op_consent(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ConsentParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let consent = ConsentFlags {
        authorized_by_adult: p.authorized_by_adult,
        recorded_at: Some(ctx.now.clone()),
        note: p.note,
    };
    exec(
        ctx.conn,
        "UPDATE people SET consent_flags_json = ?2 WHERE id = ?1",
        (&p.person_id, serde_json::to_string(&consent)?),
    )?;
    bump_version(ctx.conn, "people", &p.person_id, None, &ctx.now)?;
    ctx.audit("people.consent", &p.person_id, json!({"authorized_by_adult": p.authorized_by_adult}))?;
    Ok(json!({"person_id": p.person_id, "consent": consent}))
}

pub fn consent_ok(conn: &openclaim_db::rusqlite::Connection, person_id: &str) -> OcResult<bool> {
    let row = require(
        query_one(conn, "SELECT consent_flags_json FROM people WHERE id = ?1", [person_id])?,
        "person",
        person_id,
    )?;
    let c: ConsentFlags = serde_json::from_value(row["consent_flags"].clone()).unwrap_or_default();
    Ok(c.authorized_by_adult)
}

fn op_history(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    ensure_person(ctx, &p.id)?;
    let claims = query_all(
        ctx.conn,
        "SELECT c.id, c.opportunity_id, o.title, c.status, c.updated_at FROM claims c JOIN opportunities o ON o.id = c.opportunity_id WHERE c.person_id = ?1 ORDER BY c.updated_at DESC",
        [&p.id],
    )?;
    let matches = query_all(
        ctx.conn,
        "SELECT opportunity_id, assessment, stale, evaluated_at FROM matches WHERE person_id = ?1",
        [&p.id],
    )?;
    let audit = query_all(
        ctx.conn,
        "SELECT actor, action, timestamp FROM audit_log WHERE redacted_subject = ?1 ORDER BY seq DESC LIMIT 100",
        [format!("person_id={}", p.id)],
    )?;
    Ok(json!({"person_id": p.id, "claims": claims, "matches": matches, "audit": audit}))
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteParams {
    pub id: String,
    /// Must be true: this permanently removes the person, their facts, claims and matches.
    pub confirm: bool,
}

fn op_delete(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DeleteParams = parse_params(p)?;
    if !p.confirm {
        return Err(OcError::invalid("pass confirm=true to permanently delete this person"));
    }
    ensure_person(ctx, &p.id)?;
    exec(ctx.conn, "DELETE FROM people WHERE id = ?1", [&p.id])?;
    ctx.audit("people.delete", &p.id, json!({}))?;
    Ok(json!({"deleted": p.id}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("people.list", "people.read", Read, "List claimant profiles (names only)", PageParams, op_list),
        op!(
            "people.show",
            "people.read",
            Read,
            "Show a profile; sensitive details need people.read_sensitive",
            IdParam,
            op_show,
            resource = "id"
        ),
        op!("people.create", "people.modify", Elevated, "Create a claimant profile", CreateParams, op_create),
        op!(
            "people.update",
            "people.modify",
            Elevated,
            "Update name or details (optimistic version)",
            UpdateParams,
            op_update
        ),
        op!("people.residences", "people.read", Read, "Residence timeline", super::Empty, op_residences),
        op!(
            "people.add_residence",
            "people.modify",
            Elevated,
            "Add a residence period",
            ResidenceParams,
            op_add_residence
        ),
        op!(
            "people.remove_residence",
            "people.modify",
            Elevated,
            "Remove a residence period",
            IdParam,
            op_remove_residence
        ),
        op!(
            "people.consent",
            "people.modify",
            Elevated,
            "Record the adult's explicit authorization",
            ConsentParams,
            op_consent
        ),
        op!(
            "people.history",
            "people.read",
            Read,
            "Claims, matches and audit entries for a person",
            IdParam,
            op_history,
            resource = "id"
        ),
        op!(
            "people.delete",
            "people.modify",
            Elevated,
            "Hard-delete a person and everything linked to them",
            DeleteParams,
            op_delete
        ),
    ]
}
