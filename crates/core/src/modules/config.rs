//! Configuration stored in the database. No key can enable a paid service or a model.

use super::{Empty, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, scalar};
use openclaim_db::rusqlite::Connection;
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub struct ConfigKey {
    pub key: &'static str,
    pub default: fn() -> Value,
    pub description: &'static str,
}

pub const KEYS: &[ConfigKey] = &[
    ConfigKey { key: "network.contact", default: || Value::Null, description: "Contact (name/email) declared in the User-Agent for sites with fair-access rules (SEC.gov)." },
    ConfigKey { key: "network.offline", default: || json!(false), description: "When true, no connector makes network requests." },
    ConfigKey { key: "vault.ocr", default: || json!(true), description: "Use a locally installed Tesseract for image OCR when available." },
    ConfigKey { key: "vault.index_text", default: || json!(true), description: "Index redacted extracted text for local full-text search." },
    ConfigKey { key: "alerts.lead_days", default: || json!([30, 14, 7, 3, 1]), description: "Days before a verified deadline to remind." },
    ConfigKey { key: "session.ttl_minutes", default: || json!(60), description: "How long an `openclaim unlock` owner session lasts." },
    ConfigKey { key: "gui.port", default: || json!(7865), description: "Localhost port for the REST API and bundled GUI." },
    ConfigKey { key: "jobs.concurrency", default: || json!(2), description: "Maximum background jobs running at once." },
    ConfigKey { key: "sources.refresh_hours", default: || json!(24), description: "Scheduled refresh interval for permitted sources." },
];

pub fn get(conn: &Connection, key: &str) -> OcResult<Value> {
    let def = KEYS.iter().find(|k| k.key == key).ok_or_else(|| OcError::invalid(format!("unknown config key '{key}'")))?;
    let stored: Option<String> = scalar(conn, "SELECT value_json FROM config WHERE key = ?1", [key])?;
    Ok(stored.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(def.default))
}

pub fn get_bool(conn: &Connection, key: &str) -> bool {
    get(conn, key).ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

pub fn get_str(conn: &Connection, key: &str) -> Option<String> {
    get(conn, key).ok().and_then(|v| v.as_str().map(str::to_string)).filter(|s| !s.trim().is_empty())
}

pub fn get_u64(conn: &Connection, key: &str, fallback: u64) -> u64 {
    get(conn, key).ok().and_then(|v| v.as_u64()).unwrap_or(fallback)
}

fn validate_value(key: &str, v: &Value) -> OcResult<()> {
    let ok = match key {
        "network.contact" => v.is_null() || v.as_str().is_some_and(|s| s.len() <= 200 && !s.contains('\n')),
        "network.offline" | "vault.ocr" | "vault.index_text" => v.is_boolean(),
        "alerts.lead_days" => v.as_array().is_some_and(|a| !a.is_empty() && a.len() <= 10 && a.iter().all(|d| d.as_u64().is_some_and(|d| (1..=365).contains(&d)))),
        "session.ttl_minutes" => v.as_u64().is_some_and(|n| (1..=1440).contains(&n)),
        "gui.port" => v.as_u64().is_some_and(|n| (1024..=65535).contains(&n)),
        "jobs.concurrency" => v.as_u64().is_some_and(|n| (1..=8).contains(&n)),
        "sources.refresh_hours" => v.as_u64().is_some_and(|n| (1..=720).contains(&n)),
        _ => false,
    };
    if ok { Ok(()) } else { Err(OcError::invalid(format!("value {v} is not valid for '{key}'"))) }
}

pub fn show_all(conn: &Connection) -> OcResult<Value> {
    let mut out = serde_json::Map::new();
    for k in KEYS {
        out.insert(k.key.to_string(), json!({"value": get(conn, k.key)?, "default": (k.default)(), "description": k.description}));
    }
    Ok(Value::Object(out))
}

#[derive(Deserialize, JsonSchema)]
pub struct KeyParam {
    pub key: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct SetParam {
    pub key: String,
    pub value: Value,
}

fn op_show(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    show_all(ctx.conn)
}

fn op_get(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: KeyParam = parse_params(p)?;
    Ok(json!({"key": p.key, "value": get(ctx.conn, &p.key)?}))
}

fn op_set(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SetParam = parse_params(p)?;
    validate_value(&p.key, &p.value)?;
    exec(ctx.conn, "INSERT INTO config(key, value_json, updated_at) VALUES (?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at", (&p.key, p.value.to_string(), &ctx.now))?;
    if p.key == "network.contact" {
        crate::modules::sources::recheck_configuration(ctx.conn, &ctx.now)?;
    }
    Ok(json!({"key": p.key, "value": p.value}))
}

fn op_validate(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let rows = query_all(ctx.conn, "SELECT key, value_json FROM config", [])?;
    let mut problems = Vec::new();
    for r in rows {
        let key = r["key"].as_str().unwrap_or("");
        if let Err(e) = validate_value(key, &r["value"]) {
            problems.push(e.message);
        }
    }
    Ok(json!({"valid": problems.is_empty(), "problems": problems}))
}

fn op_reset(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: KeyParam = parse_params(p)?;
    if p.key == "*" {
        exec(ctx.conn, "DELETE FROM config", [])?;
    } else {
        exec(ctx.conn, "DELETE FROM config WHERE key = ?1", [&p.key])?;
    }
    show_all(ctx.conn)
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("config.show", "daemon.read", Read, "Show every configuration key with its value, default and description", Empty, op_show),
        op!("config.get", "daemon.read", Read, "Get one configuration value", KeyParam, op_get),
        op!("config.set", "config.write", Elevated, "Set a configuration value", SetParam, op_set),
        op!("config.validate", "daemon.read", Read, "Validate stored configuration", Empty, op_validate),
        op!("config.reset", "config.write", Elevated, "Reset a key (or '*' for all) to defaults", KeyParam, op_reset),
    ]
}
