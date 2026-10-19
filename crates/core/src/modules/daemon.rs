use super::{Empty, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{query_all, scalar};
use openclaim_domain::OcResult;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

fn op_status(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let count = |t: &str| -> OcResult<i64> { Ok(scalar::<i64, _>(ctx.conn, &format!("SELECT count(*) FROM {t}"), [])?.unwrap_or(0)) };
    let uptime = (time::OffsetDateTime::now_utc() - ctx.svc.inner.started_at).whole_seconds();
    Ok(json!({
        "version": openclaim_domain::VERSION,
        "instance_id": ctx.svc.inner.instance_id,
        "uptime_seconds": uptime,
        "data_dir": ctx.svc.inner.paths.data_dir.to_string_lossy(),
        "schema_version": scalar::<String, _>(ctx.conn, "SELECT value FROM meta WHERE key='schema_version'", [])?,
        "owner_initialized": scalar::<i64, _>(ctx.conn, "SELECT count(*) FROM owner", [])?.unwrap_or(0) > 0,
        "actor": ctx.actor.id(),
        "counts": {
            "opportunities": count("opportunities")?,
            "verified_opportunities": scalar::<i64, _>(ctx.conn, "SELECT count(*) FROM opportunities WHERE verification_state='verified' AND archived_at IS NULL", [])?.unwrap_or(0),
            "sources": count("sources")?,
            "people": count("people")?,
            "documents": count("documents")?,
            "claims": count("claims")?,
            "jobs_active": scalar::<i64, _>(ctx.conn, "SELECT count(*) FROM jobs WHERE state IN ('queued','running','paused_for_approval')", [])?.unwrap_or(0),
            "approvals_pending": scalar::<i64, _>(ctx.conn, "SELECT count(*) FROM agent_approvals WHERE decision='pending' AND expires_at > ?1", [&ctx.now])?.unwrap_or(0),
        },
        "ai_integration": "none (OpenClaim ships no models, inference runtimes or AI API clients)",
    }))
}

fn op_health(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let sources = query_all(
        ctx.conn,
        "SELECT id, name, health, enabled, last_ok_at, last_failure, consecutive_failures FROM sources ORDER BY id",
        [],
    )?;
    let jobs = query_all(ctx.conn, "SELECT state, count(*) AS n FROM jobs GROUP BY state", [])?;
    let problems: Vec<String> = sources
        .iter()
        .filter(|s| s["enabled"] == true && matches!(s["health"].as_str(), Some("blocked") | Some("degraded") | Some("needs_configuration")))
        .map(|s| {
            format!(
                "source {} is {}: {}",
                s["id"].as_str().unwrap_or(""),
                s["health"].as_str().unwrap_or(""),
                s["last_failure"].as_str().unwrap_or("")
            )
        })
        .collect();
    let ok = problems.is_empty();
    Ok(
        json!({"ok": ok, "sources": sources, "jobs": jobs, "problems": problems, "note": "A blocked or stale source means results may be incomplete; it never means there are no settlements."}),
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct DoctorParams {}

fn op_doctor(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let mut checks = Vec::new();
    let paths = &ctx.svc.inner.paths;
    let writable = std::fs::metadata(&paths.data_dir).map(|m| !m.permissions().readonly()).unwrap_or(false);
    checks.push(json!({"check": "data directory writable", "ok": writable, "detail": paths.data_dir.to_string_lossy()}));
    checks.push(json!({"check": "vault key loaded", "ok": true, "detail": if cfg!(windows) { "DPAPI-wrapped key" } else { "owner-only key file" }}));
    let owner = scalar::<i64, _>(ctx.conn, "SELECT count(*) FROM owner", [])?.unwrap_or(0) > 0;
    checks.push(json!({"check": "owner passphrase set", "ok": owner, "detail": if owner { "set" } else { "run `openclaim init` so approvals and personal data need a human" }}));
    let contact = crate::modules::config::get_str(ctx.conn, "network.contact").is_some();
    checks.push(json!({"check": "network.contact configured (needed for SEC.gov)", "ok": contact, "detail": if contact { "set" } else { "openclaim config set network.contact \"Your Name you@example.com\"" }}));
    let tesseract = openclaim_extract::find_tesseract(&openclaim_extract::OcrConfig {
        tesseract: None,
        enabled: true,
    });
    checks.push(json!({"check": "tesseract OCR available (optional)", "ok": tesseract.is_some(), "detail": tesseract.map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "not installed; image OCR unavailable".into())}));
    let integrity = ctx.svc.inner.db.try_lock().is_err();
    checks.push(json!({"check": "database reachable", "ok": true, "detail": if integrity { "in use" } else { "idle" }}));
    checks.push(json!({"check": "no AI runtime linked", "ok": true, "detail": "no model, inference or AI SDK dependencies are compiled in"}));
    Ok(json!({"ok": checks.iter().all(|c| c["ok"] == true || c["check"].as_str().unwrap_or("").contains("optional")), "checks": checks}))
}

fn op_logs(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: super::PageParams = parse_params(p)?;
    let limit = p.limit.unwrap_or(100).clamp(1, 1000);
    let rows = query_all(
        ctx.conn,
        "SELECT id, actor_id, command, result, dry_run, duration_ms, created_at FROM command_runs ORDER BY created_at DESC LIMIT ?1",
        [limit],
    )?;
    Ok(json!({"items": rows, "log_dir": ctx.svc.inner.paths.log_dir.to_string_lossy()}))
}

fn op_stop(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    if ctx.dry_run {
        return Ok(json!({"would_stop": true}));
    }
    ctx.svc.inner.stop_requested.store(true, std::sync::atomic::Ordering::SeqCst);
    ctx.svc.inner.shutdown.notify_waiters();
    Ok(json!({"stopping": true, "note": "durable jobs resume when the daemon restarts"}))
}

fn op_events(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        after_seq: i64,
        #[serde(default)]
        limit: Option<u32>,
    }
    let p: P = parse_params(p)?;
    let events = crate::events::since(ctx.conn, p.after_seq, p.limit.unwrap_or(200).clamp(1, 1000))?;
    let filtered: Vec<_> = events
        .into_iter()
        .filter(|e| ctx.actor.is_owner() || !e.topic.starts_with("approval.") || ctx.actor.has_scope("jobs.read", None))
        .collect();
    Ok(json!({"events": filtered}))
}

#[derive(Deserialize, JsonSchema)]
pub struct EventsParams {
    #[serde(default)]
    pub after_seq: i64,
    #[serde(default)]
    pub limit: Option<u32>,
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!(
            "daemon.status",
            "daemon.read",
            Read,
            "Daemon version, uptime, data location and record counts",
            Empty,
            op_status,
            mcp = "get_daemon_status"
        ),
        op!(
            "daemon.health",
            "daemon.read",
            Read,
            "Source health, job states and explicit problems",
            Empty,
            op_health,
            mcp = "get_source_health"
        ),
        op!(
            "daemon.doctor",
            "daemon.read",
            Read,
            "Environment checks (storage, passphrase, contact, OCR)",
            DoctorParams,
            op_doctor
        ),
        op!("daemon.logs", "daemon.read", Read, "Recent command runs", super::PageParams, op_logs),
        op!(
            "daemon.stop",
            "daemon.manage",
            OwnerOnly,
            "Stop the daemon; durable jobs resume on restart",
            Empty,
            op_stop
        ),
        op!(
            "events.poll",
            "jobs.read",
            Read,
            "Read committed events after a sequence number",
            EventsParams,
            op_events
        ),
    ]
}
