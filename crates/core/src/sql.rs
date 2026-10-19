//! Small helpers that turn rows into JSON objects so modules stay compact.

use openclaim_db::DbResultExt;
use openclaim_db::rusqlite::types::ValueRef;
use openclaim_db::rusqlite::{Connection, Params, Row};
use openclaim_domain::{OcError, OcResult};
use serde_json::{Map, Value};

pub fn row_json(row: &Row<'_>) -> Result<Value, openclaim_db::rusqlite::Error> {
    let mut obj = Map::new();
    let stmt = row.as_ref();
    for (i, name) in stmt.column_names().iter().enumerate() {
        let v = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => {
                if name.ends_with("_json") {
                    Value::from(n)
                } else if is_bool_column(name) {
                    Value::Bool(n != 0)
                } else {
                    Value::from(n)
                }
            }
            ValueRef::Real(f) => Value::from(f),
            ValueRef::Text(t) => {
                let s = String::from_utf8_lossy(t).into_owned();
                if name.ends_with("_json") {
                    serde_json::from_str(&s).unwrap_or(Value::String(s))
                } else {
                    Value::String(s)
                }
            }
            ValueRef::Blob(b) => Value::String(format!("<{} bytes>", b.len())),
        };
        let key = name.strip_suffix("_json").unwrap_or(name);
        obj.insert(key.to_string(), v);
    }
    Ok(Value::Object(obj))
}

fn is_bool_column(name: &str) -> bool {
    matches!(
        name,
        "enabled" | "bookmarked" | "hidden" | "deadline_verified" | "free_only" | "positive" | "asserted_by_user" | "validated" | "stale"
            | "automated_search" | "heir_claim" | "dry_run" | "cancel_requested" | "follow_detail_pages" | "automation_allowed"
            | "user_confirmed" | "terms_confirmed"
    )
}

pub fn query_all<P: Params>(conn: &Connection, sql: &str, params: P) -> OcResult<Vec<Value>> {
    let mut stmt = conn.prepare(sql).db()?;
    let rows = stmt.query_map(params, row_json).db()?;
    rows.map(|r| r.db()).collect()
}

pub fn query_one<P: Params>(conn: &Connection, sql: &str, params: P) -> OcResult<Option<Value>> {
    let mut stmt = conn.prepare(sql).db()?;
    let mut rows = stmt.query(params).db()?;
    match rows.next().db()? {
        Some(row) => Ok(Some(row_json(row).db()?)),
        None => Ok(None),
    }
}

pub fn scalar<T: openclaim_db::rusqlite::types::FromSql, P: Params>(conn: &Connection, sql: &str, params: P) -> OcResult<Option<T>> {
    use openclaim_db::rusqlite::OptionalExtension;
    conn.query_row(sql, params, |r| r.get::<_, T>(0)).optional().db()
}

pub fn exec<P: Params>(conn: &Connection, sql: &str, params: P) -> OcResult<usize> {
    conn.execute(sql, params).db()
}

pub fn require<T>(v: Option<T>, what: &str, id: &str) -> OcResult<T> {
    v.ok_or_else(|| OcError::not_found(format!("{what} '{id}' not found")))
}

/// Escapes a user query for FTS5 MATCH by quoting each token.
pub fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .filter(|t| t.len() > 2)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn like_pattern(s: &str) -> String {
    format!("%{}%", s.replace('%', "\\%").replace('_', "\\_"))
}

/// Base64url offset cursor with a snapshot timestamp so pages never mix refreshes.
pub fn encode_cursor(offset: u64, snapshot: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{offset}|{snapshot}"))
}

pub fn decode_cursor(cursor: &str) -> OcResult<(u64, String)> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cursor).map_err(|_| OcError::invalid("malformed cursor"))?;
    let s = String::from_utf8(bytes).map_err(|_| OcError::invalid("malformed cursor"))?;
    let (o, snap) = s.split_once('|').ok_or_else(|| OcError::invalid("malformed cursor"))?;
    Ok((o.parse().map_err(|_| OcError::invalid("malformed cursor"))?, snap.to_string()))
}
