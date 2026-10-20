pub mod agents;
pub mod alerts;
pub mod claims;
pub mod config;
pub mod daemon;
pub mod data;
pub mod documents;
pub mod facts;
pub mod jobs_ops;
pub mod matching;
pub mod opportunities;
pub mod people;
pub mod schema;
pub mod sources;
pub mod workflows;

use openclaim_domain::envelope::Pagination;
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct IdParam {
    pub id: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct PageParams {
    /// Page size (1-500). Default 50.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Empty {}

/// Offset pagination pinned to a snapshot timestamp so refreshes never shift pages.
pub struct Pager {
    pub limit: u32,
    pub offset: u64,
    pub snapshot: String,
}

impl Pager {
    pub fn from(p: &PageParams, now: &str) -> OcResult<Pager> {
        let limit = p.limit.unwrap_or(50).clamp(1, 500);
        match &p.cursor {
            Some(c) if !c.is_empty() => {
                let (offset, snapshot) = crate::sql::decode_cursor(c)?;
                Ok(Pager { limit, offset, snapshot })
            }
            _ => Ok(Pager {
                limit,
                offset: 0,
                snapshot: now.to_string(),
            }),
        }
    }

    /// Wraps `rows` (fetched with LIMIT limit+1) into `{items, pagination}`.
    pub fn wrap(&self, mut rows: Vec<Value>, total: Option<u64>) -> Value {
        let has_more = rows.len() as u32 > self.limit;
        rows.truncate(self.limit as usize);
        let next = has_more.then(|| crate::sql::encode_cursor(self.offset + self.limit as u64, &self.snapshot));
        json!({
            "items": rows,
            "pagination": Pagination { limit: self.limit, next_cursor: next, total, snapshot: Some(self.snapshot.clone()) },
        })
    }

    pub fn sql_limit(&self) -> String {
        format!(" LIMIT {} OFFSET {}", self.limit + 1, self.offset)
    }
}

pub fn parse_params<T: serde::de::DeserializeOwned>(v: Value) -> OcResult<T> {
    let v = if v.is_null() { Value::Object(Default::default()) } else { v };
    serde_json::from_value(v).map_err(|e| OcError::invalid(format!("invalid parameters: {e}")))
}

pub fn bump_version(conn: &openclaim_db::rusqlite::Connection, table: &str, id: &str, expected: Option<i64>, now: &str) -> OcResult<i64> {
    let current: Option<i64> = crate::sql::scalar(conn, &format!("SELECT version FROM {table} WHERE id = ?1"), [id])?;
    let current = current.ok_or_else(|| OcError::not_found(format!("{} '{id}' not found", table.trim_end_matches('s'))))?;
    if let Some(e) = expected
        && e != current
    {
        return Err(OcError::conflict(format!("stale version: expected {e}, current {current}"))
            .with_next_action("re-read the record and retry with the current version"));
    }
    crate::sql::exec(
        conn,
        &format!("UPDATE {table} SET version = version + 1, updated_at = ?2 WHERE id = ?1"),
        (id, now),
    )?;
    Ok(current + 1)
}

pub fn opt_date(s: &Option<String>) -> OcResult<Option<String>> {
    match s.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(d) => {
            openclaim_domain::dates::parse_date(d)?;
            Ok(Some(d.to_string()))
        }
    }
}
