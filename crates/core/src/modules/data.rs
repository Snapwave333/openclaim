//! Backup, restore, export, import, integrity and wipe (MDR §12, §14).

use super::parse_params;
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, scalar};
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Deserialize, JsonSchema)]
pub struct PathParam {
    pub path: String,
}

fn op_backup(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PathParam = parse_params(p)?;
    let dest = Path::new(&p.path);
    if dest.exists() {
        return Err(OcError::conflict(format!("{} already exists", dest.display())));
    }
    if ctx.dry_run {
        return Ok(json!({"would_write": p.path}));
    }
    {
        let mut dest_conn = openclaim_db::rusqlite::Connection::open(dest).map_err(openclaim_db::db_err)?;
        let backup = openclaim_db::rusqlite::backup::Backup::new(ctx.conn, &mut dest_conn).map_err(openclaim_db::db_err)?;
        backup.run_to_completion(256, std::time::Duration::from_millis(5), None).map_err(openclaim_db::db_err)?;
    }
    let vault_dir = dest.with_extension("vault");
    std::fs::create_dir_all(&vault_dir).map_err(|e| OcError::internal(e.to_string()))?;
    let mut files = 0;
    for entry in std::fs::read_dir(&ctx.svc.inner.paths.vault_dir).map_err(|e| OcError::internal(e.to_string()))?.flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), vault_dir.join(entry.file_name())).map_err(|e| OcError::internal(e.to_string()))?;
            files += 1;
        }
    }
    ctx.audit("data.backup", &p.path, json!({"vault_files": files}))?;
    Ok(json!({"database": p.path, "vault_dir": vault_dir.to_string_lossy(), "vault_files": files, "note": "vault files stay encrypted with this machine's key; keep the key file (vault.key.dpapi) with the backup for another machine"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct RestoreParams {
    pub path: String,
    pub confirm: bool,
}

/// Executed by the dispatcher outside a transaction (see `Service::restore`).
fn op_restore(_ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Err(OcError::internal("data.restore is handled by the dispatcher"))
}

fn decrypt_opt(ctx: &Ctx<'_>, v: &Value) -> Value {
    match v.as_str() {
        Some(s) => ctx.svc.vault().decrypt_str(s).ok().and_then(|j| serde_json::from_str(&j).ok().or(Some(Value::String(j)))).unwrap_or(Value::Null),
        None => Value::Null,
    }
}

pub fn export_all(ctx: &Ctx<'_>) -> OcResult<Value> {
    let mut people = query_all(ctx.conn, "SELECT * FROM people", [])?;
    for p in &mut people {
        let d = decrypt_opt(ctx, &p["encrypted_optional_details"]);
        p["details"] = d;
        p.as_object_mut().map(|o| o.remove("encrypted_optional_details"));
    }
    let mut claims = query_all(ctx.conn, "SELECT * FROM claims", [])?;
    for c in &mut claims {
        let n = decrypt_opt(ctx, &c["notes_enc"]);
        c["notes"] = n;
        c.as_object_mut().map(|o| o.remove("notes_enc"));
    }
    let mut docs = query_all(ctx.conn, "SELECT id, checksum, doc_type, mime, size_bytes, encrypted_metadata, extraction_state, person_id, imported_at FROM documents", [])?;
    for d in &mut docs {
        let m = decrypt_opt(ctx, &d["encrypted_metadata"]);
        d["metadata"] = m;
        d.as_object_mut().map(|o| o.remove("encrypted_metadata"));
    }
    let mut tracking = query_all(ctx.conn, "SELECT * FROM property_tracking", [])?;
    for t in &mut tracking {
        let n = decrypt_opt(ctx, &t["notes_enc"]);
        t["notes"] = n;
        t.as_object_mut().map(|o| o.remove("notes_enc"));
    }
    Ok(json!({
        "format": "openclaim-export",
        "schema_version": 1,
        "exported_at": ctx.now,
        "people": people,
        "residence_periods": query_all(ctx.conn, "SELECT * FROM residence_periods", [])?,
        "consumer_facts": query_all(ctx.conn, "SELECT * FROM consumer_facts", [])?,
        "attestations": query_all(ctx.conn, "SELECT * FROM attestations", [])?,
        "class_exclusions": query_all(ctx.conn, "SELECT * FROM class_exclusions", [])?,
        "claims": claims,
        "claim_events": query_all(ctx.conn, "SELECT * FROM claim_events", [])?,
        "ledger": query_all(ctx.conn, "SELECT * FROM ledger", [])?,
        "documents": docs,
        "document_links": query_all(ctx.conn, "SELECT * FROM document_links", [])?,
        "matches": query_all(ctx.conn, "SELECT * FROM matches", [])?,
        "alerts": query_all(ctx.conn, "SELECT * FROM alerts", [])?,
        "property_tracking": tracking,
        "bookmarks": query_all(ctx.conn, "SELECT * FROM opportunity_marks", [])?,
        "opportunities": query_all(ctx.conn, "SELECT o.*, (SELECT json_group_array(json_object('assertion_type', e.assertion_type, 'locator', e.precise_locator, 'excerpt', e.excerpt)) FROM evidence_links e WHERE e.opportunity_id = o.id) AS provenance_json FROM opportunities o WHERE o.id IN (SELECT opportunity_id FROM claims UNION SELECT opportunity_id FROM opportunity_marks WHERE bookmarked = 1 UNION SELECT opportunity_id FROM matches WHERE assessment = 'potential_match')", [])?,
    }))
}

#[derive(Deserialize, JsonSchema)]
pub struct ExportParams {
    /// json | csv | markdown
    #[serde(default)]
    pub format: Option<String>,
    /// Directory (csv) or file path (json/markdown) to write to; omitted = inline.
    #[serde(default)]
    pub path: Option<String>,
}

fn op_export(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ExportParams = parse_params(p)?;
    if !ctx.actor.is_owner() {
        return Err(OcError::denied("full exports are owner-only"));
    }
    let data = export_all(ctx)?;
    let format = p.format.as_deref().unwrap_or("json");
    let written: Vec<String>;
    match format {
        "json" => {
            if let Some(path) = &p.path {
                if !ctx.dry_run {
                    std::fs::write(path, serde_json::to_string_pretty(&data)?).map_err(|e| OcError::invalid(format!("write {path}: {e}")))?;
                }
                written = vec![path.clone()];
            } else {
                return Ok(data);
            }
        }
        "csv" => {
            let dir = p.path.as_deref().ok_or_else(|| OcError::invalid("csv export needs path (a directory)"))?;
            if !ctx.dry_run {
                std::fs::create_dir_all(dir).map_err(|e| OcError::invalid(e.to_string()))?;
            }
            let tables: &[(&str, &[&str])] = &[
                ("claims", &["id", "person_id", "opportunity_id", "status", "submitted_at", "external_confirmation_id", "manual_paid_amount", "currency", "updated_at"]),
                ("consumer_facts", &["id", "person_id", "fact_type", "merchant", "product", "start_date", "end_date", "positive", "proof_id"]),
                ("matches", &["person_id", "opportunity_id", "assessment", "rules_version", "evaluated_at"]),
                ("ledger", &["id", "claim_id", "amount_minor", "currency", "received_on", "note"]),
                ("opportunities", &["id", "title", "status", "verification_state", "claim_deadline", "official_url", "claim_url"]),
            ];
            let mut w = Vec::new();
            for (t, cols) in tables {
                let items = data[*t].as_array().cloned().unwrap_or_default();
                let file = Path::new(dir).join(format!("{t}.csv"));
                if !ctx.dry_run {
                    std::fs::write(&file, crate::modules::opportunities::to_csv(&items, cols)).map_err(|e| OcError::invalid(e.to_string()))?;
                }
                w.push(file.to_string_lossy().to_string());
            }
            written = w;
        }
        "markdown" => {
            let mut md = format!("# OpenClaim export ({})\n\n", ctx.now);
            md.push_str("## Claims\n\n");
            md.push_str(&crate::modules::opportunities::to_markdown(data["claims"].as_array().unwrap_or(&vec![]), &["id", "opportunity_id", "status", "external_confirmation_id", "manual_paid_amount", "currency"]));
            md.push_str("\n## Tracked opportunities\n\n");
            md.push_str(&crate::modules::opportunities::to_markdown(data["opportunities"].as_array().unwrap_or(&vec![]), &["title", "status", "verification_state", "claim_deadline", "official_url"]));
            md.push_str("\n## Ledger\n\n");
            md.push_str(&crate::modules::opportunities::to_markdown(data["ledger"].as_array().unwrap_or(&vec![]), &["received_on", "amount_minor", "currency", "note"]));
            if let Some(path) = &p.path {
                if !ctx.dry_run {
                    std::fs::write(path, &md).map_err(|e| OcError::invalid(format!("write {path}: {e}")))?;
                }
                written = vec![path.clone()];
            } else {
                return Ok(json!({"format": "markdown", "content": md}));
            }
        }
        other => return Err(OcError::invalid(format!("unknown format '{other}'"))),
    }
    ctx.audit("data.export", "all", json!({"format": format, "files": written}))?;
    Ok(json!({"format": format, "written": written}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ImportParams {
    /// Path to a JSON export produced by data.export.
    pub path: String,
}

fn op_import(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ImportParams = parse_params(p)?;
    let text = std::fs::read_to_string(&p.path).map_err(|e| OcError::invalid(format!("read {}: {e}", p.path)))?;
    if text.len() > 200 * 1024 * 1024 {
        return Err(OcError::invalid("export file is larger than 200 MB"));
    }
    let data: Value = serde_json::from_str(&text)?;
    if data["format"] != "openclaim-export" {
        return Err(OcError::invalid("not an OpenClaim export"));
    }
    let mut counts = serde_json::Map::new();
    let mut n_people = 0;
    for person in data["people"].as_array().unwrap_or(&vec![]) {
        let id = person["id"].as_str().unwrap_or("");
        if id.is_empty() || scalar::<String, _>(ctx.conn, "SELECT id FROM people WHERE id = ?1", [id])?.is_some() {
            continue;
        }
        let details = ctx.svc.vault().encrypt_str(&person["details"].to_string());
        exec(ctx.conn, "INSERT INTO people(id, display_name, encrypted_optional_details, consent_flags_json, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?5)", (id, person["display_name"].as_str().unwrap_or("Imported"), details, person["consent_flags"].to_string(), &ctx.now))?;
        n_people += 1;
    }
    counts.insert("people".into(), json!(n_people));
    let mut n = 0;
    for r in data["residence_periods"].as_array().unwrap_or(&vec![]) {
        n += exec(ctx.conn, "INSERT OR IGNORE INTO residence_periods(id, person_id, jurisdiction, start_date, end_date, provenance, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", (r["id"].as_str(), r["person_id"].as_str(), r["jurisdiction"].as_str(), r["start_date"].as_str(), r["end_date"].as_str(), "imported", &ctx.now))?;
    }
    counts.insert("residence_periods".into(), json!(n));
    let mut n = 0;
    for f in data["consumer_facts"].as_array().unwrap_or(&vec![]) {
        n += exec(ctx.conn, "INSERT OR IGNORE INTO consumer_facts(id, person_id, fact_type, merchant, product, start_date, end_date, positive, asserted_by_user, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,?9)", (f["id"].as_str(), f["person_id"].as_str(), f["fact_type"].as_str(), f["merchant"].as_str(), f["product"].as_str(), f["start_date"].as_str(), f["end_date"].as_str(), f["positive"].as_bool().unwrap_or(true) as i32, &ctx.now))?;
    }
    counts.insert("consumer_facts".into(), json!(n));
    let mut n = 0;
    for a in data["attestations"].as_array().unwrap_or(&vec![]) {
        n += exec(ctx.conn, "INSERT OR IGNORE INTO attestations(person_id, opportunity_id, key, value, recorded_at) VALUES (?1,?2,?3,?4,?5)", (a["person_id"].as_str(), a["opportunity_id"].as_str(), a["key"].as_str(), a["value"].as_i64().unwrap_or(0), &ctx.now))?;
    }
    counts.insert("attestations".into(), json!(n));
    let mut n = 0;
    for c in data["claims"].as_array().unwrap_or(&vec![]) {
        let oid = c["opportunity_id"].as_str().unwrap_or("");
        if scalar::<String, _>(ctx.conn, "SELECT id FROM opportunities WHERE id = ?1", [oid])?.is_none() {
            // Re-create the catalog record from the export so the claim keeps its context.
            if let Some(o) = data["opportunities"].as_array().and_then(|a| a.iter().find(|o| o["id"] == oid)) {
                exec(ctx.conn, "INSERT OR IGNORE INTO opportunities(id, source_id, external_key, dedupe_key, title, opp_type, official_url, status, verification_state, authority_tier, claim_deadline, first_seen_at, updated_at) VALUES (?1,NULL,NULL,?2,?3,?4,?5,?6,'verification_pending',?7,?8,?9,?9)", (oid, o["dedupe_key"].as_str().unwrap_or(""), o["title"].as_str().unwrap_or("imported"), o["opp_type"].as_str().unwrap_or("other"), o["official_url"].as_str(), o["status"].as_str().unwrap_or("unknown"), o["authority_tier"].as_str().unwrap_or("B"), o["claim_deadline"].as_str(), &ctx.now))?;
                crate::modules::sources::reindex(ctx.conn, oid)?;
            } else {
                continue;
            }
        }
        let notes = c["notes"].as_str().map(|s| ctx.svc.vault().encrypt_str(s));
        n += exec(ctx.conn, "INSERT OR IGNORE INTO claims(id, person_id, opportunity_id, status, submitted_at, external_confirmation_id, manual_paid_amount, currency, notes_enc, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)", (c["id"].as_str(), c["person_id"].as_str(), oid, c["status"].as_str().unwrap_or("discovered"), c["submitted_at"].as_str(), c["external_confirmation_id"].as_str(), c["manual_paid_amount"].as_i64(), c["currency"].as_str(), notes, &ctx.now))?;
    }
    counts.insert("claims".into(), json!(n));
    let mut n = 0;
    for e in data["claim_events"].as_array().unwrap_or(&vec![]) {
        n += exec(ctx.conn, "INSERT OR IGNORE INTO claim_events(id, claim_id, event_type, from_status, to_status, who_recorded, timestamp, evidence_id, metadata) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", (e["id"].as_str(), e["claim_id"].as_str(), e["event_type"].as_str().unwrap_or("imported"), e["from_status"].as_str(), e["to_status"].as_str(), e["who_recorded"].as_str().unwrap_or("import"), e["timestamp"].as_str().unwrap_or(&ctx.now), e["evidence_id"].as_str(), e["metadata"].to_string()))?;
    }
    counts.insert("claim_events".into(), json!(n));
    let mut n = 0;
    for t in data["property_tracking"].as_array().unwrap_or(&vec![]) {
        let notes = t["notes"].as_str().map(|s| ctx.svc.vault().encrypt_str(s));
        n += exec(ctx.conn, "INSERT OR IGNORE INTO property_tracking(id, person_id, jurisdiction, status, heir_claim, notes_enc, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", (t["id"].as_str(), t["person_id"].as_str(), t["jurisdiction"].as_str(), t["status"].as_str().unwrap_or("to_search"), t["heir_claim"].as_bool().unwrap_or(false) as i32, notes, &ctx.now))?;
    }
    counts.insert("property_tracking".into(), json!(n));
    crate::jobs::enqueue(ctx, "match_refresh", json!({}), None)?;
    ctx.audit("data.import", &p.path, json!(counts))?;
    Ok(json!({"imported": counts, "note": "documents are not imported from JSON; restore the vault directory from a backup for those"}))
}

fn op_integrity(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let fk = query_all(ctx.conn, "PRAGMA foreign_key_check", [])?;
    let quick: String = scalar(ctx.conn, "PRAGMA quick_check", [])?.unwrap_or_else(|| "unknown".into());
    let (count, broken) = crate::audit::verify(ctx.conn)?;
    let orphans: i64 = scalar(ctx.conn, "SELECT count(*) FROM documents WHERE vault_path NOT LIKE '%.bin'", [])?.unwrap_or(0);
    let missing_files = query_all(ctx.conn, "SELECT id, vault_path FROM documents", [])?.into_iter().filter(|d| !Path::new(d["vault_path"].as_str().unwrap_or("")).exists()).count();
    let last: Option<String> = scalar(ctx.conn, "SELECT value FROM meta WHERE key = 'last_integrity_check'", [])?;
    Ok(json!({"ok": quick == "ok" && fk.is_empty() && broken.is_none() && missing_files == 0, "quick_check": quick, "foreign_key_violations": fk.len(), "audit_entries": count, "audit_broken_at": broken, "documents_missing_files": missing_files, "odd_paths": orphans, "last_scheduled_check": last, "recommendation": "run `openclaim data backup <path>` now if you have not recently"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct WipeParams {
    /// Must be the literal string WIPE.
    pub confirm: String,
    /// Also remove the downloaded catalog and source snapshots.
    #[serde(default)]
    pub include_catalog: bool,
}

fn op_wipe(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: WipeParams = parse_params(p)?;
    if p.confirm != "WIPE" {
        return Err(OcError::invalid("pass confirm=\"WIPE\" to erase local personal data"));
    }
    let personal = ["claim_packages", "claim_events", "ledger", "claims", "matches", "attestations", "class_exclusions", "proposed_facts", "consumer_facts", "residence_periods", "document_links", "documents_fts", "documents", "property_events", "property_tracking", "people", "alerts", "data_disclosures", "agent_approvals", "agent_sessions", "agent_principals", "idempotency_keys", "workflow_runs", "workflows", "job_events", "jobs", "opportunity_marks", "opportunity_reports", "tips"];
    let mut rows = 0;
    for t in personal {
        rows += exec(ctx.conn, &format!("DELETE FROM {t}"), [])?;
    }
    if p.include_catalog {
        for t in ["evidence_links", "opportunity_changes", "opportunity_aliases", "opportunity_rules", "opportunities_fts", "opportunities", "source_snapshots", "events", "command_runs"] {
            rows += exec(ctx.conn, &format!("DELETE FROM {t}"), [])?;
        }
        exec(ctx.conn, "UPDATE sources SET etag = NULL, last_modified = NULL, next_fetch_at = NULL, last_ok_at = NULL, health = 'never_fetched'", [])?;
    }
    let mut files = 0;
    if !ctx.dry_run {
        if let Ok(rd) = std::fs::read_dir(&ctx.svc.inner.paths.vault_dir) {
            for e in rd.flatten() {
                if e.path().is_file() && std::fs::remove_file(e.path()).is_ok() {
                    files += 1;
                }
            }
        }
        ctx.svc.inner.sessions.lock().map_err(|_| OcError::internal("lock"))?.retain(|_, s| s.transport == "system");
    }
    ctx.audit("data.wipe", "all", json!({"rows": rows, "files": files, "include_catalog": p.include_catalog}))?;
    Ok(json!({"rows_deleted": rows, "vault_files_deleted": files, "include_catalog": p.include_catalog, "note": "the owner passphrase and audit trail are kept; run `openclaim init` is not needed"}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("data.backup", "data.export", Elevated, "Consistent copy of the database plus encrypted vault files", PathParam, op_backup),
        op!("data.restore", "data.wipe", OwnerOnly, "Replace the live database with a backup", RestoreParams, op_restore),
        op!("data.export", "data.export", Elevated, "Export your records (json, csv, markdown)", ExportParams, op_export),
        op!("data.import", "data.wipe", OwnerOnly, "Import a JSON export", ImportParams, op_import),
        op!("data.integrity", "daemon.read", Read, "Database, vault and audit-chain checks", super::Empty, op_integrity),
        op!("data.wipe", "data.wipe", OwnerOnly, "Erase local personal data (confirm=WIPE)", WipeParams, op_wipe),
    ]
}
