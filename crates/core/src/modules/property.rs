//! Unclaimed-property assistant (MDR §5.6): official directory, private
//! per-person checklists, user-entered results. No bulk name searches.

use super::{PageParams, Pager, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require, scalar};
use openclaim_db::rusqlite::Connection;
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const BUNDLED_DIRECTORY: &str = include_str!("../../../../test-fixtures/html/naupa_search.html");
const BUNDLED_CAPTURED_AT: &str = "2026-10-19T15:23:00Z";

pub fn instructions_for(kind: &str) -> &'static str {
    match kind {
        "us_state" | "us_territory" => "Open the official site, search your name (and previous names) yourself, then record any match here. Claims are filed with the state; OpenClaim never submits them.",
        "multi_state" => "Searches several states at once but not all; still check each state you lived or worked in.",
        "federal" => "Guidance for federal programs (tax refunds, pensions, savings bonds). Follow the official links.",
        "canada" => "Canadian programs listed by NAUPA for people who lived in Canada.",
        _ => "Official program listed by NAUPA.",
    }
}

pub fn seed(conn: &Connection) -> OcResult<()> {
    let n: i64 = scalar(conn, "SELECT count(*) FROM property_programs", [])?.unwrap_or(0);
    if n > 0 {
        return Ok(());
    }
    let (snapshot_id, _) = crate::modules::sources::record_snapshot(conn, "naupa-directory", "https://unclaimed.org/search/", BUNDLED_DIRECTORY, "fixture", Some(200), BUNDLED_CAPTURED_AT)?;
    let mut programs = openclaim_discovery::parse::naupa::parse_directory(BUNDLED_DIRECTORY, "https://unclaimed.org/search/");
    programs.extend(openclaim_discovery::parse::naupa::fixed_programs());
    crate::modules::sources::ingest(conn, "naupa-directory", &snapshot_id, openclaim_domain::model::AuthorityTier::A, &[], &programs, &[], BUNDLED_CAPTURED_AT)?;
    Ok(())
}

#[derive(Deserialize, JsonSchema)]
pub struct StatesParams {
    /// us_state, us_territory, multi_state, federal, canada, other
    #[serde(default)]
    pub kind: Option<String>,
}

fn op_states(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: StatesParams = parse_params(p)?;
    let rows = match p.kind {
        Some(k) => query_all(ctx.conn, "SELECT jurisdiction, name, official_url, kind, automated_search, instructions, verified_at FROM property_programs WHERE kind = ?1 ORDER BY jurisdiction", [k])?,
        None => query_all(ctx.conn, "SELECT jurisdiction, name, official_url, kind, automated_search, instructions, verified_at FROM property_programs ORDER BY kind, jurisdiction", [])?,
    };
    Ok(json!({"items": rows, "note": "No program here offers a free bulk API; searches are done by you on the official site."}))
}

#[derive(Deserialize, JsonSchema)]
pub struct LinksParams {
    pub jurisdictions: Vec<String>,
}

fn op_official_links(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: LinksParams = parse_params(p)?;
    let mut items = Vec::new();
    let mut unknown = Vec::new();
    for j in &p.jurisdictions {
        match query_one(ctx.conn, "SELECT jurisdiction, name, official_url, kind, instructions, verified_at FROM property_programs WHERE jurisdiction = ?1", [j.to_uppercase()])? {
            Some(r) => items.push(r),
            None => unknown.push(j.clone()),
        }
    }
    items.extend(query_all(ctx.conn, "SELECT jurisdiction, name, official_url, kind, instructions, verified_at FROM property_programs WHERE kind = 'multi_state'", [])?);
    Ok(json!({"items": items, "unknown": unknown}))
}

#[derive(Deserialize, JsonSchema)]
pub struct PersonParam {
    pub person_id: String,
}

fn jurisdictions_for(conn: &Connection, person_id: &str) -> OcResult<Vec<String>> {
    let rows = query_all(conn, "SELECT DISTINCT upper(jurisdiction) AS j FROM residence_periods WHERE person_id = ?1 ORDER BY j", [person_id])?;
    Ok(rows.into_iter().filter_map(|r| r["j"].as_str().map(str::to_string)).collect())
}

fn op_checklist(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PersonParam = parse_params(p)?;
    crate::modules::people::ensure_person(ctx, &p.person_id)?;
    let js = jurisdictions_for(ctx.conn, &p.person_id)?;
    let mut steps = Vec::new();
    for j in &js {
        let prog = query_one(ctx.conn, "SELECT jurisdiction, name, official_url, instructions FROM property_programs WHERE jurisdiction = ?1", [j])?;
        let tracking = query_one(ctx.conn, "SELECT status, updated_at FROM property_tracking WHERE person_id = ?1 AND jurisdiction = ?2", (&p.person_id, j))?;
        steps.push(json!({"jurisdiction": j, "program": prog, "status": tracking.as_ref().map(|t| t["status"].clone()).unwrap_or(json!("to_search")), "updated_at": tracking.map(|t| t["updated_at"].clone())}));
    }
    let multi = query_all(ctx.conn, "SELECT jurisdiction, name, official_url, instructions FROM property_programs WHERE kind IN ('multi_state','federal')", [])?;
    Ok(json!({
        "person_id": p.person_id,
        "steps": steps,
        "also_check": multi,
        "note": if js.is_empty() { "Add residence periods to the profile to build a state checklist." } else { "A checklist, not a completed nationwide search. Search each site yourself and record results." },
    }))
}

fn op_research(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PersonParam = parse_params(p)?;
    crate::modules::people::ensure_person(ctx, &p.person_id)?;
    let js = jurisdictions_for(ctx.conn, &p.person_id)?;
    let mut created = 0;
    for j in &js {
        let id = openclaim_db::new_id();
        let n = exec(ctx.conn, "INSERT OR IGNORE INTO property_tracking(id, person_id, jurisdiction, status, updated_at) VALUES (?1,?2,?3,'to_search',?4)", (&id, &p.person_id, j, &ctx.now))?;
        created += n;
    }
    Ok(json!({"person_id": p.person_id, "jurisdictions": js, "created": created}))
}

#[derive(Deserialize, JsonSchema)]
pub struct TrackParams {
    pub person_id: String,
    pub jurisdiction: String,
    /// to_search, searched_no_match, possible_match, claim_started, claim_submitted, paid, not_mine
    pub status: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub heir_claim: Option<bool>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

fn op_track(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: TrackParams = parse_params(p)?;
    crate::modules::people::ensure_person(ctx, &p.person_id)?;
    const STATUSES: &[&str] = &["to_search", "searched_no_match", "possible_match", "claim_started", "claim_submitted", "paid", "not_mine"];
    if !STATUSES.contains(&p.status.as_str()) {
        return Err(OcError::invalid(format!("status must be one of {}", STATUSES.join(", "))));
    }
    let j = p.jurisdiction.to_uppercase();
    let existing = query_one(ctx.conn, "SELECT id, version FROM property_tracking WHERE person_id = ?1 AND jurisdiction = ?2", (&p.person_id, &j))?;
    let note_enc = p.note.as_deref().map(|n| ctx.svc.vault().encrypt_str(n));
    let id = match existing {
        Some(row) => {
            let id = row["id"].as_str().unwrap_or("").to_string();
            super::bump_version(ctx.conn, "property_tracking", &id, p.expected_version, &ctx.now)?;
            exec(ctx.conn, "UPDATE property_tracking SET status = ?2, heir_claim = COALESCE(?3, heir_claim), notes_enc = COALESCE(?4, notes_enc) WHERE id = ?1", (&id, &p.status, p.heir_claim.map(|b| b as i32), note_enc))?;
            id
        }
        None => {
            let id = openclaim_db::new_id();
            exec(ctx.conn, "INSERT INTO property_tracking(id, person_id, jurisdiction, status, heir_claim, notes_enc, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", (&id, &p.person_id, &j, &p.status, p.heir_claim.unwrap_or(false) as i32, note_enc, &ctx.now))?;
            id
        }
    };
    exec(ctx.conn, "INSERT INTO property_events(id, tracking_id, status, recorded_by, at) VALUES (?1,?2,?3,?4,?5)", (openclaim_db::new_id(), &id, &p.status, ctx.actor_id(), &ctx.now))?;
    Ok(json!({"tracking_id": id, "jurisdiction": j, "status": p.status}))
}

fn op_history(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        person_id: String,
        #[serde(flatten)]
        page: PageParams,
    }
    let p: P = parse_params(p)?;
    crate::modules::people::ensure_person(ctx, &p.person_id)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let mut rows = query_all(ctx.conn, &format!("SELECT t.id, t.jurisdiction, t.status, t.heir_claim, t.notes_enc, t.updated_at, t.version FROM property_tracking t WHERE t.person_id = ?1 AND t.updated_at <= ?2 ORDER BY t.jurisdiction{}", pager.sql_limit()), (&p.person_id, &pager.snapshot))?;
    for r in &mut rows {
        let note = r["notes_enc"].as_str().and_then(|n| ctx.svc.vault().decrypt_str(n).ok());
        r["note"] = if ctx.actor.is_owner() { json!(note) } else { json!(note.map(|_| "[hidden from agents]")) };
        r.as_object_mut().map(|o| o.remove("notes_enc"));
        let id = r["id"].as_str().unwrap_or("").to_string();
        r["events"] = json!(query_all(ctx.conn, "SELECT status, recorded_by, at FROM property_events WHERE tracking_id = ?1 ORDER BY at", [&id])?);
    }
    Ok(pager.wrap(rows, None))
}

fn op_heir_checklist(_ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({
        "title": "Claiming unclaimed property as an heir",
        "steps": [
            "Confirm you have legal authority: executor/administrator letters, small-estate affidavit, or being a named heir under the state's rules.",
            "Gather the decedent's full name, former addresses, dates, and a certified death certificate.",
            "Search each state the decedent lived or worked in; record possible matches here.",
            "Open the official claim on the state site yourself; expect to upload proof of identity and authority.",
            "Keep every confirmation number and letter; record them as claim events.",
        ],
        "note": "Requirements differ by state and are set by the state, not OpenClaim. This is organizational help, not legal advice.",
        "ok": true
    }))
}

pub fn require_program(conn: &Connection, jurisdiction: &str) -> OcResult<Value> {
    require(query_one(conn, "SELECT * FROM property_programs WHERE jurisdiction = ?1", [jurisdiction])?, "program", jurisdiction)
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("property.states", "property.read", Read, "Official unclaimed-property programs (50 states, DC, territories, federal, multi-state)", StatesParams, op_states, mcp = "list_property_programs"),
        op!("property.official_links", "property.read", Read, "Official search links for given jurisdictions", LinksParams, op_official_links),
        op!("property.checklist", "property.track", Read, "Private per-person search checklist from residence history", PersonParam, op_checklist),
        op!("property.research", "property.track", Write, "Create to-search tracking rows for every state in the residence history", PersonParam, op_research),
        op!("property.track", "property.track", Write, "Record a user-entered search result or claim status", TrackParams, op_track),
        op!("property.history", "property.track", Read, "Tracking rows and events for a person", PersonParam, op_history),
        op!("property.heir_checklist", "property.read", Read, "Guided checklist for heirs' claims", super::Empty, op_heir_checklist),
    ]
}
