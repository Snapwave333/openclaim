use super::{PageParams, Pager, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require};
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

fn visible(ctx: &Ctx<'_>) -> Option<String> {
    // Agents see only their own jobs and system jobs they may read; owners see all.
    if ctx.actor.is_owner() { None } else { Some(ctx.actor_id()) }
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        state: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(flatten)]
        page: PageParams,
    }
    let p: P = parse_params(p)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let owner = visible(ctx);
    let rows = query_all(
        ctx.conn,
        &format!("SELECT id, kind, state, progress, owner_actor_id, parent_job_id, next_run_at, attempts, max_attempts, error_code, error_message, created_at, updated_at, finished_at FROM jobs
                  WHERE created_at <= ?1 AND (?2 IS NULL OR state = ?2) AND (?3 IS NULL OR kind = ?3) AND (?4 IS NULL OR owner_actor_id = ?4) ORDER BY created_at DESC{}", pager.sql_limit()),
        (&pager.snapshot, &p.state, &p.kind, &owner),
    )?;
    Ok(pager.wrap(rows, None))
}

#[derive(Deserialize, JsonSchema)]
pub struct JobIdParam {
    pub job_id: String,
}

pub fn load(ctx: &Ctx<'_>, id: &str) -> OcResult<Value> {
    let row = require(query_one(ctx.conn, "SELECT id, kind, state, progress, params_json, result_json, owner_actor_id, parent_job_id, next_run_at, attempts, max_attempts, cancel_requested, error_code, error_message, created_at, updated_at, finished_at FROM jobs WHERE id = ?1", [id])?, "job", id)?;
    if let Some(o) = visible(ctx) {
        if row["owner_actor_id"] != o {
            return Err(OcError::not_found(format!("job '{id}' not found")));
        }
    }
    let mut row = row;
    if let Some(r) = row["result"].as_str() {
        row["result"] = serde_json::from_str(r).unwrap_or(Value::String(r.to_string()));
    }
    row["next_action"] = json!(match row["state"].as_str() {
        Some("paused_for_approval") => "ask the owner to decide the approval, then `openclaim jobs retry <id>`",
        Some("failed") => "inspect error_message; `openclaim jobs retry <id>` if transient",
        Some("queued") | Some("running") => "`openclaim jobs watch <id>`",
        _ => "none",
    });
    Ok(row)
}

fn op_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: JobIdParam = parse_params(p)?;
    load(ctx, &p.job_id)
}

#[derive(Deserialize, JsonSchema)]
pub struct EnqueueParams {
    pub kind: String,
    #[serde(default)]
    pub params: Value,
}

fn op_enqueue(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: EnqueueParams = parse_params(p)?;
    if p.kind == "workflow_run" {
        return Err(OcError::invalid("use workflows.run"));
    }
    if matches!(p.kind.as_str(), "source_refresh" | "source_test") {
        ctx.require_scope("sources.refresh", None)?;
    }
    let id = crate::jobs::enqueue(ctx, &p.kind, p.params, None)?;
    Ok(json!({"job_id": id, "status": "queued"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct WatchParams {
    pub job_id: String,
    #[serde(default)]
    pub after_sequence: i64,
}

fn op_watch(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: WatchParams = parse_params(p)?;
    let job = load(ctx, &p.job_id)?;
    let events = query_all(ctx.conn, "SELECT sequence, event_type AS event, payload_redacted_json, at FROM job_events WHERE job_id = ?1 AND sequence > ?2 ORDER BY sequence", (&p.job_id, p.after_sequence))?;
    let terminal = matches!(job["state"].as_str(), Some("succeeded") | Some("failed") | Some("cancelled"));
    Ok(json!({"job": job, "events": events, "terminal": terminal}))
}

fn op_cancel(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: JobIdParam = parse_params(p)?;
    let job = load(ctx, &p.job_id)?;
    match job["state"].as_str() {
        Some("queued") | Some("paused_for_approval") => {
            exec(ctx.conn, "UPDATE jobs SET state = 'cancelled', cancel_requested = 1, finished_at = ?2, updated_at = ?2 WHERE id = ?1", (&p.job_id, &ctx.now))?;
            crate::jobs::job_event(ctx.conn, &p.job_id, "job.cancelled", json!({}), &ctx.now)?;
            exec(ctx.conn, "UPDATE workflow_runs SET state = 'cancelled', updated_at = ?2 WHERE job_id = ?1", (&p.job_id, &ctx.now))?;
            Ok(json!({"job_id": p.job_id, "state": "cancelled"}))
        }
        Some("running") => {
            exec(ctx.conn, "UPDATE jobs SET cancel_requested = 1, updated_at = ?2 WHERE id = ?1", (&p.job_id, &ctx.now))?;
            Ok(json!({"job_id": p.job_id, "state": "running", "cancel_requested": true, "note": "stops at the next cancellation point"}))
        }
        other => Err(OcError::conflict(format!("job is {}", other.unwrap_or("unknown")))),
    }
}

fn op_retry(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: JobIdParam = parse_params(p)?;
    let job = load(ctx, &p.job_id)?;
    match job["state"].as_str() {
        Some("failed") | Some("cancelled") | Some("paused_for_approval") => {
            exec(ctx.conn, "UPDATE jobs SET state = 'queued', cancel_requested = 0, next_run_at = ?2, error_code = NULL, error_message = NULL, finished_at = NULL, updated_at = ?2, lease_owner = NULL WHERE id = ?1", (&p.job_id, &ctx.now))?;
            exec(ctx.conn, "UPDATE workflow_runs SET state = 'queued', updated_at = ?2 WHERE job_id = ?1", (&p.job_id, &ctx.now))?;
            crate::jobs::job_event(ctx.conn, &p.job_id, "job.requeued", json!({}), &ctx.now)?;
            ctx.jobs_enqueued = true;
            Ok(json!({"job_id": p.job_id, "state": "queued"}))
        }
        other => Err(OcError::conflict(format!("job is {}; only failed, cancelled or paused jobs can be retried", other.unwrap_or("unknown")))),
    }
}

fn op_history(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: JobIdParam = parse_params(p)?;
    load(ctx, &p.job_id)?;
    Ok(json!({"job_id": p.job_id, "events": query_all(ctx.conn, "SELECT sequence, event_type AS event, payload_redacted_json, at FROM job_events WHERE job_id = ?1 ORDER BY sequence", [&p.job_id])?}))
}

fn op_kinds(_ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({"kinds": crate::jobs::KINDS.iter().map(|(k, d)| json!({"kind": k, "description": d})).collect::<Vec<_>>()}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("jobs.list", "jobs.read", Read, "List jobs (agents see their own)", PageParams, op_list),
        op!("jobs.show", "jobs.read", Read, "Job state, progress, result and next action", JobIdParam, op_show, mcp = "get_job"),
        op!("jobs.enqueue", "jobs.manage", Write, "Queue a job by kind", EnqueueParams, op_enqueue),
        op!("jobs.watch", "jobs.read", Read, "Job plus events after a sequence (poll for streaming)", WatchParams, op_watch, mcp = "watch_job"),
        op!("jobs.cancel", "jobs.manage", Write, "Cancel a queued/paused job or request cancellation of a running one", JobIdParam, op_cancel),
        op!("jobs.retry", "jobs.manage", Write, "Requeue a failed, cancelled or paused job", JobIdParam, op_retry),
        op!("jobs.history", "jobs.read", Read, "All events for a job", JobIdParam, op_history),
        op!("jobs.kinds", "jobs.read", Read, "Known job kinds", super::Empty, op_kinds),
    ]
}
