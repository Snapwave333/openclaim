use openclaim_db::rusqlite::Connection;
use openclaim_domain::OcResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub topic: String,
    pub payload: Value,
    pub at: String,
}

/// Writes to the transactional outbox. The dispatcher broadcasts after commit.
pub fn emit(conn: &Connection, topic: &str, payload: Value) -> OcResult<i64> {
    let at = openclaim_domain::dates::now_rfc3339();
    crate::sql::exec(
        conn,
        "INSERT INTO events(topic, payload, at) VALUES (?1, ?2, ?3)",
        (topic, payload.to_string(), &at),
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn since(conn: &Connection, after_seq: i64, limit: u32) -> OcResult<Vec<Event>> {
    let rows = crate::sql::query_all(
        conn,
        "SELECT seq, topic, payload, at FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        (after_seq, limit),
    )?;
    Ok(rows
        .into_iter()
        .map(|r| Event {
            seq: r["seq"].as_i64().unwrap_or(0),
            topic: r["topic"].as_str().unwrap_or("").to_string(),
            payload: r["payload"].as_str().and_then(|p| serde_json::from_str(p).ok()).unwrap_or(Value::Null),
            at: r["at"].as_str().unwrap_or("").to_string(),
        })
        .collect())
}
