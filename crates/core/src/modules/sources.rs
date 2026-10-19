//! Source registry, refresh jobs, snapshot import and ingestion with dedupe,
//! provenance and change detection (MDR §4, §5.8).

use super::{Empty, PageParams, Pager, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require, scalar};
use openclaim_db::rusqlite::Connection;
use openclaim_discovery::manifest::{AuthRequirement, ConnectorManifest};
use openclaim_discovery::parse::{ParsedOpportunity, ParsedProgram, ParsedTip};
use openclaim_domain::model::{AuthorityTier, FetchFailure, OpportunityStatus, SourceHealth, VerificationState, dedupe_key, same_settlement};
use openclaim_domain::{OcError, OcResult};
use openclaim_security::digest::sha256_hex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn seed(conn: &Connection) -> OcResult<()> {
    let now = openclaim_domain::dates::now_rfc3339();
    for m in openclaim_discovery::manifest::bundled() {
        let existing: Option<String> = scalar(conn, "SELECT manifest_json FROM sources WHERE id = ?1", [&m.id])?;
        let manifest_json = serde_json::to_string(&m)?;
        match existing {
            None => {
                exec(
                    conn,
                    "INSERT INTO sources(id, name, url, connector, authority_tier, manifest_json, enabled, health, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,1,'never_fetched',?7)",
                    (&m.id, &m.name, &m.url, serde_json::to_value(m.connector)?.as_str().unwrap_or(""), m.authority_tier.as_str(), &manifest_json, &now),
                )?;
            }
            Some(old) if old != manifest_json => {
                exec(conn, "UPDATE sources SET name = ?2, url = ?3, manifest_json = ?4 WHERE id = ?1", (&m.id, &m.name, &m.url, &manifest_json))?;
            }
            _ => {}
        }
    }
    recheck_configuration(conn, &now)
}

/// Sources needing a declared contact flip between needs_configuration and ready.
pub fn recheck_configuration(conn: &Connection, _now: &str) -> OcResult<()> {
    let contact = crate::modules::config::get_str(conn, "network.contact");
    for row in query_all(conn, "SELECT id, manifest_json, health FROM sources WHERE enabled = 1", [])? {
        let m: ConnectorManifest = serde_json::from_value(row["manifest"].clone())?;
        let id = row["id"].as_str().unwrap_or("");
        let needs = m.auth_requirement == AuthRequirement::DeclaredContactUserAgent && contact.is_none();
        let health = row["health"].as_str().unwrap_or("");
        if needs && health != "needs_configuration" {
            exec(conn, "UPDATE sources SET health = 'needs_configuration', last_failure = 'policy_blocked', last_failure_detail = 'set network.contact first' WHERE id = ?1", [id])?;
        } else if !needs && health == "needs_configuration" {
            exec(conn, "UPDATE sources SET health = 'never_fetched', last_failure = NULL, last_failure_detail = NULL WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

pub fn manifest_of(conn: &Connection, source_id: &str) -> OcResult<ConnectorManifest> {
    let row = require(query_one(conn, "SELECT manifest_json FROM sources WHERE id = ?1", [source_id])?, "source", source_id)?;
    Ok(serde_json::from_value(row["manifest"].clone())?)
}

pub fn record_snapshot(conn: &Connection, source_id: &str, url: &str, body: &str, acquisition: &str, status: Option<u16>, now: &str) -> OcResult<(String, bool)> {
    let hash = sha256_hex(body.as_bytes());
    let prev: Option<String> = scalar(conn, "SELECT content_hash FROM source_snapshots WHERE source_id = ?1 AND url = ?2 ORDER BY fetched_at DESC LIMIT 1", (source_id, url))?;
    let changed = prev.as_deref() != Some(&hash);
    let id = openclaim_db::new_id();
    let excerpt = openclaim_security::sanitize::clean_text(&openclaim_security::sanitize::html_fragment_to_text(body), 600);
    exec(
        conn,
        "INSERT INTO source_snapshots(id, source_id, url, content_hash, fetched_at, acquisition, http_status, change_summary, parser_version, excerpt)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        (&id, source_id, url, &hash, now, acquisition, status.map(|s| s as i64), if changed { if prev.is_some() { "content changed" } else { "first capture" } } else { "unchanged" }, openclaim_discovery::parse::PARSER_VERSION, excerpt),
    )?;
    Ok((id, changed))
}

#[derive(Debug, Default, Serialize)]
pub struct IngestSummary {
    pub inserted: u32,
    pub updated: u32,
    pub merged: u32,
    pub changes: u32,
    pub programs: u32,
    pub tips: u32,
}

const WATCHED_FIELDS: &[&str] = &["status", "claim_deadline", "claim_url", "official_url", "title", "summary", "class_period_start", "class_period_end", "administrator", "verification_state"];

pub fn ingest(conn: &Connection, source_id: &str, snapshot_id: &str, tier: AuthorityTier, records: &[ParsedOpportunity], programs: &[ParsedProgram], tips: &[ParsedTip], now: &str) -> OcResult<IngestSummary> {
    let mut s = IngestSummary::default();
    for r in records {
        let existing = query_one(conn, "SELECT id FROM opportunities WHERE source_id = ?1 AND external_key = ?2", (source_id, &r.external_key))?
            .or(query_one(conn, "SELECT opportunity_id AS id FROM opportunity_aliases WHERE source_id = ?1 AND external_key = ?2", (source_id, &r.external_key))?);
        let key = dedupe_key(&r.title, r.case_number.as_deref(), r.administrator.as_deref(), r.class_period_start.as_deref());
        if let Some(ex) = existing {
            let id = ex["id"].as_str().unwrap_or("").to_string();
            let changed = update_opportunity(conn, &id, r, tier, snapshot_id, now)?;
            s.changes += changed;
            s.updated += 1;
            continue;
        }
        let title_part = key.split('|').next().unwrap_or("").to_string();
        let candidates = if title_part.is_empty() { Vec::new() } else { query_all(conn, "SELECT id, dedupe_key FROM opportunities WHERE dedupe_key LIKE ?1 ESCAPE '\\' AND archived_at IS NULL", [format!("{}|%", title_part.replace('%', "\\%").replace('_', "\\_"))])? };
        if let Some(c) = candidates.iter().find(|c| same_settlement(c["dedupe_key"].as_str().unwrap_or(""), &key)) {
            let id = c["id"].as_str().unwrap_or("").to_string();
            exec(conn, "INSERT OR IGNORE INTO opportunity_aliases(opportunity_id, source_id, external_key, title) VALUES (?1,?2,?3,?4)", (&id, source_id, &r.external_key, &r.title))?;
            let changed = update_opportunity(conn, &id, r, tier, snapshot_id, now)?;
            s.changes += changed;
            s.merged += 1;
            continue;
        }
        insert_opportunity(conn, source_id, r, tier, &key, snapshot_id, now)?;
        s.inserted += 1;
    }
    for p in programs {
        exec(
            conn,
            "INSERT INTO property_programs(jurisdiction, name, official_url, kind, snapshot_id, automated_search, instructions, verified_at)
             VALUES (?1,?2,?3,?4,?5,0,?6,?7)
             ON CONFLICT(jurisdiction) DO UPDATE SET name = excluded.name, official_url = excluded.official_url, kind = excluded.kind, snapshot_id = excluded.snapshot_id, verified_at = excluded.verified_at",
            (&p.jurisdiction, &p.name, &p.official_url, &p.kind, snapshot_id, crate::modules::property::instructions_for(&p.kind), now),
        )?;
        s.programs += 1;
    }
    for t in tips {
        let exists: Option<String> = scalar(conn, "SELECT id FROM tips WHERE url = ?1", [&t.url])?;
        if exists.is_none() {
            exec(conn, "INSERT INTO tips(id, url, title, note, submitted_by, state, created_at) VALUES (?1,?2,?3,?4,?5,'quarantined',?6)", (openclaim_db::new_id(), &t.url, &t.title, t.summary.clone(), format!("source:{source_id}"), now))?;
            s.tips += 1;
        }
    }
    Ok(s)
}

fn verification_for(tier: AuthorityTier, terms_confirmed: bool) -> VerificationState {
    if tier == AuthorityTier::A && terms_confirmed { VerificationState::Verified } else { VerificationState::VerificationPending }
}

fn insert_opportunity(conn: &Connection, source_id: &str, r: &ParsedOpportunity, tier: AuthorityTier, key: &str, snapshot_id: &str, now: &str) -> OcResult<String> {
    let id = openclaim_db::new_id();
    let vs = verification_for(tier, r.terms_confirmed);
    exec(
        conn,
        "INSERT INTO opportunities(id, source_id, external_key, dedupe_key, title, opp_type, administrator, administrator_phone, case_number, official_url, claim_url,
            status, verification_state, authority_tier, claim_deadline, deadline_verified, class_period_start, class_period_end, jurisdictions_json, summary, benefit_json,
            proof_required, claim_method, source_updated_on, first_seen_at, updated_at, verified_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?25,?26)",
        openclaim_db::rusqlite::params![
            id, source_id, r.external_key, key, r.title, r.opp_type.as_str(), r.administrator, r.administrator_phone, r.case_number, r.official_url, r.claim_url,
            r.status.as_str(), vs.as_str(), tier.as_str(), r.claim_deadline, r.claim_deadline.is_some() as i32, r.class_period_start, r.class_period_end,
            serde_json::to_string(&r.jurisdictions)?, r.summary, serde_json::to_string(&r.benefit)?, r.proof_required.as_str(), r.claim_method, r.source_updated_on, now,
            if vs == VerificationState::Verified { Some(now) } else { None }
        ],
    )?;
    reindex(conn, &id)?;
    add_evidence(conn, &id, snapshot_id, r, now)?;
    crate::events::emit(conn, "opportunity.new", json!({"opportunity_id": id, "title": r.title, "verification_state": vs.as_str()}))?;
    Ok(id)
}

/// Applies a parsed record to an existing row. Index rows never erase detail-page facts.
fn update_opportunity(conn: &Connection, id: &str, r: &ParsedOpportunity, tier: AuthorityTier, snapshot_id: &str, now: &str) -> OcResult<u32> {
    let before = require(query_one(conn, "SELECT * FROM opportunities WHERE id = ?1", [id])?, "opportunity", id)?;
    let was_verified = before["verification_state"] == "verified";
    let vs = if r.terms_confirmed { verification_for(tier, true) } else if was_verified { VerificationState::Verified } else { VerificationState::VerificationPending };
    if r.terms_confirmed {
        exec(
            conn,
            "UPDATE opportunities SET title = ?2, administrator = COALESCE(?3, administrator), administrator_phone = COALESCE(?4, administrator_phone), case_number = COALESCE(?5, case_number),
                official_url = ?6, claim_url = ?7, status = ?8, verification_state = ?9, claim_deadline = ?10, deadline_verified = ?11, class_period_start = ?12, class_period_end = ?13,
                summary = ?14, benefit_json = ?15, proof_required = ?16, claim_method = ?17, source_updated_on = COALESCE(?18, source_updated_on), updated_at = ?19,
                verified_at = CASE WHEN ?9 = 'verified' THEN ?19 ELSE verified_at END, archived_at = NULL
             WHERE id = ?1",
            openclaim_db::rusqlite::params![
                id, r.title, r.administrator, r.administrator_phone, r.case_number, r.official_url, r.claim_url, r.status.as_str(), vs.as_str(), r.claim_deadline,
                r.claim_deadline.is_some() as i32, r.class_period_start, r.class_period_end, r.summary, serde_json::to_string(&r.benefit)?, r.proof_required.as_str(), r.claim_method,
                r.source_updated_on, now
            ],
        )?;
    } else {
        exec(
            conn,
            "UPDATE opportunities SET title = ?2, administrator = COALESCE(?3, administrator), administrator_phone = COALESCE(?4, administrator_phone), case_number = COALESCE(?5, case_number),
                source_updated_on = COALESCE(?6, source_updated_on), updated_at = ?7, archived_at = NULL,
                verified_at = CASE WHEN verification_state = 'verified' THEN ?7 ELSE verified_at END
             WHERE id = ?1",
            openclaim_db::rusqlite::params![id, r.title, r.administrator, r.administrator_phone, r.case_number, r.source_updated_on, now],
        )?;
    }
    if before["verification_state"] == "stale" && vs == VerificationState::Verified {
        exec(conn, "UPDATE opportunities SET verification_state = 'verified' WHERE id = ?1", [id])?;
    }
    let after = require(query_one(conn, "SELECT * FROM opportunities WHERE id = ?1", [id])?, "opportunity", id)?;
    let mut changes = 0;
    for f in WATCHED_FIELDS {
        if before[f] != after[f] {
            changes += 1;
            exec(
                conn,
                "INSERT INTO opportunity_changes(id, opportunity_id, field, old_value, new_value, snapshot_id, detected_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                (openclaim_db::new_id(), id, f, value_text(&before[f]), value_text(&after[f]), snapshot_id, now),
            )?;
        }
    }
    if changes > 0 {
        exec(conn, "UPDATE opportunities SET version = version + 1 WHERE id = ?1", [id])?;
        exec(conn, "UPDATE matches SET stale = 1 WHERE opportunity_id = ?1", [id])?;
        reindex(conn, id)?;
        crate::events::emit(conn, "opportunity.changed", json!({"opportunity_id": id, "title": r.title, "changes": changes}))?;
        if before["claim_deadline"] != after["claim_deadline"] || before["status"] != after["status"] {
            crate::modules::alerts::change_alert(conn, id, &r.title, &format!("Official terms changed: {}", changed_fields(&before, &after).join(", ")), now)?;
        }
    }
    add_evidence(conn, id, snapshot_id, r, now)?;
    Ok(changes)
}

fn changed_fields(before: &Value, after: &Value) -> Vec<String> {
    WATCHED_FIELDS.iter().filter(|f| before[**f] != after[**f]).map(|f| f.to_string()).collect()
}

fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn add_evidence(conn: &Connection, id: &str, snapshot_id: &str, r: &ParsedOpportunity, now: &str) -> OcResult<()> {
    for e in &r.evidence {
        exec(
            conn,
            "INSERT INTO evidence_links(id, opportunity_id, source_snapshot_id, precise_locator, assertion_type, excerpt, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            (openclaim_db::new_id(), id, snapshot_id, &e.locator, &e.assertion_type, openclaim_security::sanitize::clean_text(&e.excerpt, 1000), now),
        )?;
    }
    Ok(())
}

pub fn reindex(conn: &Connection, id: &str) -> OcResult<()> {
    let row = require(query_one(conn, "SELECT title, administrator, summary, case_number FROM opportunities WHERE id = ?1", [id])?, "opportunity", id)?;
    exec(conn, "DELETE FROM opportunities_fts WHERE opportunity_id = ?1", [id])?;
    exec(
        conn,
        "INSERT INTO opportunities_fts(opportunity_id, title, administrator, summary, case_number) VALUES (?1,?2,?3,?4,?5)",
        (id, row["title"].as_str().unwrap_or(""), row["administrator"].as_str().unwrap_or(""), row["summary"].as_str().unwrap_or(""), row["case_number"].as_str().unwrap_or("")),
    )?;
    Ok(())
}

/// Verified records whose source has not been reachable for 3× its TTL become stale.
pub fn mark_stale(conn: &Connection, now: &str) -> OcResult<u32> {
    let mut n = 0;
    for src in query_all(conn, "SELECT id, manifest_json, last_ok_at FROM sources", [])? {
        let m: ConnectorManifest = serde_json::from_value(src["manifest"].clone())?;
        let id = src["id"].as_str().unwrap_or("");
        let limit = time::OffsetDateTime::now_utc() - time::Duration::hours(3 * m.freshness_ttl_hours as i64);
        let last_ok = src["last_ok_at"].as_str().and_then(|s| openclaim_domain::dates::parse_ts(s).ok());
        let stale = match last_ok {
            Some(t) => t < limit,
            None => true,
        };
        if stale {
            n += exec(conn, "UPDATE opportunities SET verification_state = 'stale', updated_at = ?2 WHERE source_id = ?1 AND verification_state = 'verified' AND verified_at < ?3", (id, now, openclaim_domain::dates::format_ts(limit)))? as u32;
        }
    }
    Ok(n)
}

pub fn record_failure(conn: &Connection, source_id: &str, class: FetchFailure, detail: &str, retry_after: Option<u64>, now: &str) -> OcResult<()> {
    let health = match class {
        FetchFailure::AntiBotBlock | FetchFailure::RobotsDisallowed => SourceHealth::Blocked,
        FetchFailure::PolicyBlocked => SourceHealth::NeedsConfiguration,
        FetchFailure::Offline => SourceHealth::Degraded,
        _ => SourceHealth::Degraded,
    };
    let failures: i64 = scalar::<i64, _>(conn, "SELECT consecutive_failures FROM sources WHERE id = ?1", [source_id])?.unwrap_or(0) + 1;
    let backoff_secs = retry_after.unwrap_or_else(|| (600u64 * 2u64.pow((failures.min(6) - 1) as u32)).min(86_400));
    let next = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::seconds(backoff_secs as i64));
    exec(
        conn,
        "UPDATE sources SET health = ?2, last_failure = ?3, last_failure_detail = ?4, consecutive_failures = ?5, last_attempt_at = ?6, next_fetch_at = ?7 WHERE id = ?1",
        (source_id, health.as_str(), class.as_str(), detail, failures, now, next),
    )?;
    crate::events::emit(conn, "source.failed", json!({"source_id": source_id, "class": class.as_str(), "detail": detail}))?;
    Ok(())
}

pub fn record_success(conn: &Connection, source_id: &str, etag: Option<String>, last_modified: Option<String>, ttl_hours: u32, now: &str) -> OcResult<()> {
    let next = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::hours(ttl_hours as i64));
    exec(
        conn,
        "UPDATE sources SET health = 'healthy', last_failure = NULL, last_failure_detail = NULL, consecutive_failures = 0, etag = COALESCE(?2, etag), last_modified = COALESCE(?3, last_modified), last_attempt_at = ?4, last_ok_at = ?4, next_fetch_at = ?5 WHERE id = ?1",
        (source_id, etag, last_modified, now, next),
    )?;
    Ok(())
}

/// External keys whose detail page has never been parsed, or was parsed long ago.
pub fn detail_due(conn: &Connection, source_id: &str, max: usize) -> OcResult<Vec<String>> {
    let rows = query_all(
        conn,
        "SELECT external_key FROM opportunities WHERE source_id = ?1 AND archived_at IS NULL
         ORDER BY CASE WHEN verification_state = 'verification_pending' THEN 0 ELSE 1 END, verified_at ASC, source_updated_on DESC LIMIT ?2",
        (source_id, max as i64),
    )?;
    Ok(rows.into_iter().filter_map(|r| r["external_key"].as_str().map(str::to_string)).collect())
}

// ------------------------------------------------------------------ ops

#[derive(Deserialize, JsonSchema)]
pub struct SourceIdParam {
    pub source_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RefreshParams {
    /// One source id; omit to refresh by scope.
    #[serde(default)]
    pub source_id: Option<String>,
    /// `official` (Tier A), `all`.
    #[serde(default)]
    pub scope: Option<String>,
    /// Ignore the freshness TTL.
    #[serde(default)]
    pub force: bool,
}

fn source_view(row: Value) -> Value {
    let mut r = row;
    if let Some(o) = r.as_object_mut() {
        o.remove("manifest");
        o.remove("etag");
        o.remove("last_modified");
    }
    r
}

fn op_list(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let rows = query_all(ctx.conn, "SELECT id, name, url, connector, authority_tier, trust_status, enabled, disabled_reason, health, last_failure, last_failure_detail, consecutive_failures, next_fetch_at, last_attempt_at, last_ok_at, created_at, version FROM sources ORDER BY authority_tier, id", [])?;
    Ok(json!({"items": rows}))
}

fn op_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SourceIdParam = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT * FROM sources WHERE id = ?1", [&p.source_id])?, "source", &p.source_id)?;
    let manifest = row["manifest"].clone();
    let counts = query_one(ctx.conn, "SELECT count(*) AS opportunities, sum(CASE WHEN verification_state='verified' THEN 1 ELSE 0 END) AS verified FROM opportunities WHERE source_id = ?1", [&p.source_id])?;
    let last_snapshot = query_one(ctx.conn, "SELECT id, url, fetched_at, acquisition, http_status, change_summary FROM source_snapshots WHERE source_id = ?1 ORDER BY fetched_at DESC LIMIT 1", [&p.source_id])?;
    Ok(json!({"source": source_view(row), "manifest": manifest, "counts": counts, "last_snapshot": last_snapshot}))
}

fn op_health(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let rows = query_all(ctx.conn, "SELECT id, name, health, enabled, last_ok_at, next_fetch_at, last_failure, last_failure_detail, consecutive_failures FROM sources ORDER BY id", [])?;
    let stale: i64 = scalar(ctx.conn, "SELECT count(*) FROM opportunities WHERE verification_state = 'stale'", [])?.unwrap_or(0);
    Ok(json!({"items": rows, "stale_opportunities": stale, "note": "Blocked or degraded sources mean coverage gaps, not an absence of settlements."}))
}

fn op_refresh(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RefreshParams = parse_params(p)?;
    if ctx.offline || crate::modules::config::get_bool(ctx.conn, "network.offline") {
        return Err(OcError::policy("offline mode: refresh would need the network").with_next_action("run without --offline and with network.offline=false"));
    }
    let ids: Vec<String> = match (&p.source_id, p.scope.as_deref()) {
        (Some(id), _) => {
            require(scalar::<String, _>(ctx.conn, "SELECT id FROM sources WHERE id = ?1", [id])?, "source", id)?;
            vec![id.clone()]
        }
        (None, Some("all")) => query_all(ctx.conn, "SELECT id FROM sources WHERE enabled = 1", [])?.into_iter().filter_map(|r| r["id"].as_str().map(str::to_string)).collect(),
        _ => query_all(ctx.conn, "SELECT id FROM sources WHERE enabled = 1 AND authority_tier = 'A'", [])?.into_iter().filter_map(|r| r["id"].as_str().map(str::to_string)).collect(),
    };
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    for id in ids {
        let row = require(query_one(ctx.conn, "SELECT enabled, health, next_fetch_at FROM sources WHERE id = ?1", [&id])?, "source", &id)?;
        if row["enabled"] != true {
            skipped.push(json!({"source_id": id, "reason": "disabled"}));
            continue;
        }
        if row["health"] == "needs_configuration" {
            skipped.push(json!({"source_id": id, "reason": "needs_configuration: set network.contact"}));
            continue;
        }
        if !p.force {
            if let Some(next) = row["next_fetch_at"].as_str() {
                if next > ctx.now.as_str() {
                    skipped.push(json!({"source_id": id, "reason": format!("fresh until {next}; use force=true to fetch now")}));
                    continue;
                }
            }
        }
        let job_id = crate::jobs::enqueue(ctx, "source_refresh", json!({"source_id": id, "force": p.force}), None)?;
        jobs.push(json!({"source_id": id, "job_id": job_id, "status": "queued"}));
    }
    Ok(json!({"jobs": jobs, "skipped": skipped, "job_id": jobs.first().and_then(|j| j["job_id"].as_str()), "status": if jobs.is_empty() { "nothing_queued" } else { "queued" }}))
}

fn op_test(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SourceIdParam = parse_params(p)?;
    manifest_of(ctx.conn, &p.source_id)?;
    if ctx.offline || crate::modules::config::get_bool(ctx.conn, "network.offline") {
        return Err(OcError::policy("offline mode: a source test needs one network request"));
    }
    let job_id = crate::jobs::enqueue(ctx, "source_test", json!({"source_id": p.source_id}), None)?;
    Ok(json!({"job_id": job_id, "status": "queued", "note": "fetches the index page only and does not persist records"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ManifestParam {
    /// Connector manifest JSON (object).
    pub manifest: Value,
}

fn op_validate(_ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ManifestParam = parse_params(p)?;
    let m: ConnectorManifest = serde_json::from_value(p.manifest)?;
    match m.validate() {
        Ok(()) => Ok(json!({"valid": true, "id": m.id, "connector": m.connector})),
        Err(e) => Ok(json!({"valid": false, "error": e})),
    }
}

fn op_register(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ManifestParam = parse_params(p)?;
    let m: ConnectorManifest = serde_json::from_value(p.manifest)?;
    m.validate()?;
    if m.authority_tier == AuthorityTier::A && !openclaim_discovery::manifest::bundled().iter().any(|b| b.id == m.id) {
        ctx.warn("user-registered sources are stored as Tier B leads until reviewed; records from them stay verification_pending");
    }
    let tier = if openclaim_discovery::manifest::bundled().iter().any(|b| b.id == m.id) { m.authority_tier } else { AuthorityTier::B };
    exec(
        ctx.conn,
        "INSERT INTO sources(id, name, url, connector, authority_tier, trust_status, manifest_json, enabled, health, created_at) VALUES (?1,?2,?3,?4,?5,'user_registered',?6,1,'never_fetched',?7)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, url = excluded.url, manifest_json = excluded.manifest_json, enabled = 1, disabled_reason = NULL",
        (&m.id, &m.name, &m.url, serde_json::to_value(m.connector)?.as_str().unwrap_or(""), tier.as_str(), serde_json::to_string(&m)?, &ctx.now),
    )?;
    recheck_configuration(ctx.conn, &ctx.now)?;
    Ok(json!({"source_id": m.id, "authority_tier": tier.as_str()}))
}

#[derive(Deserialize, JsonSchema)]
pub struct DisableParams {
    pub source_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

fn op_disable(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DisableParams = parse_params(p)?;
    let n = exec(ctx.conn, "UPDATE sources SET enabled = 0, disabled_reason = ?2, health = 'disabled', version = version + 1 WHERE id = ?1", (&p.source_id, p.reason.clone().unwrap_or_else(|| "disabled by owner".into())))?;
    if n == 0 {
        return Err(OcError::not_found(format!("source '{}' not found", p.source_id)));
    }
    Ok(json!({"source_id": p.source_id, "enabled": false, "note": "previously verified data is retained with a freshness warning"}))
}

fn op_enable(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SourceIdParam = parse_params(p)?;
    manifest_of(ctx.conn, &p.source_id)?;
    exec(ctx.conn, "UPDATE sources SET enabled = 1, disabled_reason = NULL, health = 'never_fetched', version = version + 1 WHERE id = ?1", [&p.source_id])?;
    recheck_configuration(ctx.conn, &ctx.now)?;
    Ok(json!({"source_id": p.source_id, "enabled": true}))
}

#[derive(Deserialize, JsonSchema)]
pub struct HistoryParams {
    pub source_id: String,
    #[serde(flatten)]
    pub page: PageParams,
}

fn op_history(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: HistoryParams = parse_params(p)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let rows = query_all(ctx.conn, &format!("SELECT id, url, content_hash, fetched_at, acquisition, http_status, change_summary, parser_version FROM source_snapshots WHERE source_id = ?1 AND fetched_at <= ?2 ORDER BY fetched_at DESC{}", pager.sql_limit()), (&p.source_id, &pager.snapshot))?;
    Ok(pager.wrap(rows, None))
}

#[derive(Deserialize, JsonSchema)]
pub struct ImportSnapshotParams {
    pub source_id: String,
    /// The official page URL the HTML was saved from.
    pub url: String,
    /// Page HTML (inline) …
    #[serde(default)]
    pub html: Option<String>,
    /// … or a local file path to read it from.
    #[serde(default)]
    pub file: Option<String>,
}

/// Human-assisted acquisition for sources that block automation (MDR §3, §4).
pub fn import_snapshot(ctx: &mut Ctx<'_>, p: ImportSnapshotParams, acquisition: &str) -> OcResult<Value> {
    let m = manifest_of(ctx.conn, &p.source_id)?;
    openclaim_security::netpolicy::check_url(&p.url, &m.allowed_hosts).map_err(|e| OcError::policy(format!("snapshot URL rejected: {e}")))?;
    let html = match (p.html, p.file) {
        (Some(h), _) => h,
        (None, Some(f)) => {
            let bytes = std::fs::read(&f).map_err(|e| OcError::invalid(format!("read {f}: {e}")))?;
            if bytes.len() > openclaim_discovery::fetch::DEFAULT_MAX_BODY {
                return Err(OcError::invalid("snapshot file is larger than 6 MB"));
            }
            String::from_utf8_lossy(&bytes).into_owned()
        }
        (None, None) => return Err(OcError::invalid("provide html or file")),
    };
    let today = openclaim_domain::dates::today_utc();
    let (opps, programs, tips) = openclaim_discovery::run::parse_page(m.connector, &p.url, &html, today).map_err(OcError::invalid)?;
    let (snapshot_id, _changed) = record_snapshot(ctx.conn, &p.source_id, &p.url, &html, acquisition, None, &ctx.now)?;
    let summary = ingest(ctx.conn, &p.source_id, &snapshot_id, m.authority_tier, &opps, &programs, &tips, &ctx.now)?;
    exec(ctx.conn, "UPDATE sources SET last_ok_at = ?2 WHERE id = ?1 AND health IN ('blocked','never_fetched','degraded')", (&p.source_id, &ctx.now))?;
    ctx.audit("sources.import_snapshot", &p.source_id, json!({"url": p.url, "acquisition": acquisition, "records": opps.len()}))?;
    Ok(json!({"snapshot_id": snapshot_id, "parsed": opps.len(), "programs": programs.len(), "tips": tips.len(), "summary": summary, "acquisition": acquisition}))
}

fn op_import_snapshot(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ImportSnapshotParams = parse_params(p)?;
    import_snapshot(ctx, p, "user_snapshot")
}

fn op_capture_from_extension(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    if !matches!(&ctx.actor, crate::ctx::Actor::Agent { principal_kind, .. } if principal_kind == "extension") && !ctx.actor.is_owner() {
        return Err(OcError::denied("only the paired browser extension or the owner can capture pages"));
    }
    let p: ImportSnapshotParams = parse_params(p)?;
    if p.file.is_some() {
        return Err(OcError::invalid("the extension must send html inline"));
    }
    import_snapshot(ctx, p, "user_snapshot")
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("sources.list", "sources.read", Read, "List registered sources with health", Empty, op_list, mcp = "list_sources"),
        op!("sources.show", "sources.read", Read, "Show a source, its manifest and last snapshot", SourceIdParam, op_show, resource = "source_id"),
        op!("sources.health", "sources.read", Read, "Freshness, failures and stale counts per source", Empty, op_health),
        op!("sources.refresh", "sources.refresh", Write, "Queue compliant refresh jobs for permitted sources", RefreshParams, op_refresh, mcp = "refresh_source", network = true),
        op!("sources.test", "sources.refresh", Write, "Queue a one-request connectivity test (nothing persisted)", SourceIdParam, op_test, network = true),
        op!("sources.validate", "sources.read", Read, "Validate a connector manifest against the free-only provider gate", ManifestParam, op_validate),
        op!("sources.register", "sources.manage", Elevated, "Register or update a connector manifest", ManifestParam, op_register),
        op!("sources.disable", "sources.manage", Elevated, "Disable a source (data retained with a freshness warning)", DisableParams, op_disable),
        op!("sources.enable", "sources.manage", Elevated, "Re-enable a source", SourceIdParam, op_enable),
        op!("sources.history", "sources.read", Read, "Snapshot history for a source", HistoryParams, op_history),
        op!("sources.import_snapshot", "sources.manage", Elevated, "Import a page you saved from the official site (for sources that block automation)", ImportSnapshotParams, op_import_snapshot),
        op!("sources.capture_from_extension", "extension.prefill", Write, "Browser extension: capture the official page the user is viewing", ImportSnapshotParams, op_capture_from_extension),
    ]
}

pub fn status_label(s: OpportunityStatus) -> &'static str {
    s.as_str()
}
