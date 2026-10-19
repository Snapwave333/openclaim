//! Hash-chained audit trail. Entries never store secrets or raw identifiers.

use crate::sql::{exec, query_all, scalar};
use openclaim_db::rusqlite::Connection;
use openclaim_domain::OcResult;
use openclaim_security::digest::sha256_hex;
use openclaim_security::redact::redact_json;
use serde_json::{Value, json};

pub fn append(conn: &Connection, actor: &str, action: &str, subject: &str, details: Value) -> OcResult<String> {
    let id = openclaim_db::new_id();
    let ts = openclaim_domain::dates::now_rfc3339();
    let prev: String = scalar(conn, "SELECT integrity_hash FROM audit_log ORDER BY seq DESC LIMIT 1", [])?.unwrap_or_else(|| "genesis".into());
    let details = redact_json(&details);
    let subject = openclaim_security::redact::redact_text(subject);
    let material = json!({"id": id, "actor": actor, "action": action, "ts": ts, "subject": subject, "details": details, "prev": prev});
    let hash = sha256_hex(openclaim_security::digest::canonical_json(&material).as_bytes());
    exec(
        conn,
        "INSERT INTO audit_log(id, actor, action, timestamp, redacted_subject, details_json, prev_hash, integrity_hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        (&id, actor, action, &ts, &subject, details.to_string(), &prev, &hash),
    )?;
    Ok(id)
}

/// Recomputes the chain; returns the first broken sequence number, if any.
pub fn verify(conn: &Connection) -> OcResult<(u64, Option<i64>)> {
    let rows = query_all(
        conn,
        "SELECT seq, id, actor, action, timestamp, redacted_subject, details_json, prev_hash, integrity_hash FROM audit_log ORDER BY seq",
        [],
    )?;
    let mut prev = "genesis".to_string();
    let mut count = 0u64;
    for r in rows {
        count += 1;
        let material = json!({"id": r["id"], "actor": r["actor"], "action": r["action"], "ts": r["timestamp"], "subject": r["redacted_subject"], "details": r["details"], "prev": prev});
        let expect = sha256_hex(openclaim_security::digest::canonical_json(&material).as_bytes());
        if r["prev_hash"].as_str() != Some(&prev) || r["integrity_hash"].as_str() != Some(&expect) {
            return Ok((count, r["seq"].as_i64()));
        }
        prev = expect;
    }
    Ok((count, None))
}
