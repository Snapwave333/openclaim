//! Declarative workflows (MDR §21): a validated DAG of allowlisted operations,
//! run as a durable job with checkpoints and approval pauses. No code execution.

use super::parse_params;
use crate::ctx::{Actor, Ctx, Request};
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require};
use crate::{Service, ops};
use openclaim_domain::scopes::Effect;
use openclaim_domain::{ErrorCode, OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Condition {
    pub step: String,
    /// Dotted path into that step's result data, e.g. `ingested.inserted`.
    pub path: String,
    pub equals: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Step {
    pub id: String,
    pub operation: String,
    #[serde(default)]
    pub with: Value,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub when: Option<Condition>,
    /// Force a human approval before this step (agents).
    #[serde(default)]
    pub approval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowDef {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub permissions: Vec<String>,
    pub steps: Vec<Step>,
}

const FORBIDDEN_SCOPES: &[&str] = &[
    "agents.manage",
    "approvals.decide",
    "data.wipe",
    "daemon.manage",
    "config.write",
    "sources.manage",
];

pub fn validate(def: &WorkflowDef) -> OcResult<Vec<String>> {
    if def.schema_version != 1 {
        return Err(OcError::invalid("schema_version must be 1"));
    }
    if def.name.trim().is_empty() || def.name.len() > 80 || !def.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(OcError::invalid("name must be [A-Za-z0-9_-]{1,80}"));
    }
    if def.steps.is_empty() || def.steps.len() > 50 {
        return Err(OcError::invalid("a workflow has 1 to 50 steps"));
    }
    for p in &def.permissions {
        if !openclaim_domain::scopes::is_known_scope(p) {
            return Err(OcError::invalid(format!("unknown permission '{p}'")));
        }
        if FORBIDDEN_SCOPES.contains(&p.as_str()) {
            return Err(OcError::policy(format!("workflows may not request '{p}'")));
        }
    }
    let ids: HashSet<&str> = def.steps.iter().map(|s| s.id.as_str()).collect();
    if ids.len() != def.steps.len() {
        return Err(OcError::invalid("step ids must be unique"));
    }
    for s in &def.steps {
        let op = ops::find(&s.operation).ok_or_else(|| OcError::invalid(format!("step '{}': unknown operation '{}'", s.id, s.operation)))?;
        if op.effect == Effect::OwnerOnly || FORBIDDEN_SCOPES.contains(&op.scope) || s.operation == "workflows.run" {
            return Err(OcError::policy(format!(
                "step '{}': operation '{}' is not allowed in workflows",
                s.id, s.operation
            )));
        }
        if !def.permissions.iter().any(|p| p == op.scope) {
            return Err(OcError::invalid(format!(
                "step '{}' needs permission '{}' which the workflow does not declare",
                s.id, op.scope
            )));
        }
        if !s.with.is_null() && !s.with.is_object() {
            return Err(OcError::invalid(format!("step '{}': with must be an object", s.id)));
        }
        for a in &s.after {
            if !ids.contains(a.as_str()) {
                return Err(OcError::invalid(format!("step '{}' waits on unknown step '{a}'", s.id)));
            }
        }
        if let Some(w) = &s.when
            && !ids.contains(w.step.as_str())
        {
            return Err(OcError::invalid(format!("step '{}' condition references unknown step '{}'", s.id, w.step)));
        }
    }
    topo_order(def)
}

pub fn topo_order(def: &WorkflowDef) -> OcResult<Vec<String>> {
    let mut deps: HashMap<&str, Vec<&str>> = HashMap::new();
    for s in &def.steps {
        let mut d: Vec<&str> = s.after.iter().map(String::as_str).collect();
        if let Some(w) = &s.when {
            d.push(w.step.as_str());
        }
        deps.insert(s.id.as_str(), d);
    }
    let mut order = Vec::new();
    let mut done: HashSet<&str> = HashSet::new();
    while order.len() < def.steps.len() {
        let before = order.len();
        for s in &def.steps {
            if done.contains(s.id.as_str()) {
                continue;
            }
            if deps[s.id.as_str()].iter().all(|d| done.contains(d)) {
                done.insert(s.id.as_str());
                order.push(s.id.clone());
            }
        }
        if order.len() == before {
            return Err(OcError::invalid("workflow steps contain a cycle"));
        }
    }
    Ok(order)
}

fn lookup(v: &Value, path: &str) -> Value {
    let mut cur = v;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        cur = match cur {
            Value::Object(m) => m.get(part).unwrap_or(&Value::Null),
            Value::Array(a) => part.parse::<usize>().ok().and_then(|i| a.get(i)).unwrap_or(&Value::Null),
            _ => &Value::Null,
        };
    }
    cur.clone()
}

/// Substitutes `${steps.<id>.data.<path>}` references in `with`.
fn bind(with: &Value, results: &BTreeMap<String, Value>) -> Value {
    match with {
        Value::String(s) if s.starts_with("${steps.") && s.ends_with('}') => {
            let inner = &s[8..s.len() - 1];
            let (step, rest) = inner.split_once('.').unwrap_or((inner, ""));
            let rest = rest.strip_prefix("data").map(|r| r.trim_start_matches('.')).unwrap_or(rest);
            results.get(step).map(|r| lookup(r, rest)).unwrap_or(Value::Null)
        }
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), bind(v, results))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(|v| bind(v, results)).collect()),
        other => other.clone(),
    }
}

pub fn plan(def: &WorkflowDef) -> OcResult<Value> {
    let order = validate(def)?;
    let steps: Vec<Value> = order
        .iter()
        .map(|id| {
            let s = def.steps.iter().find(|s| &s.id == id).unwrap();
            let op = ops::find(&s.operation).unwrap();
            json!({
                "id": s.id,
                "operation": s.operation,
                "scope": op.scope,
                "effect": op.effect,
                "network": op.network,
                "approval_gate": s.approval || op.effect == Effect::Elevated,
                "after": s.after,
                "when": s.when,
            })
        })
        .collect();
    let sources: Vec<&str> = def
        .steps
        .iter()
        .filter(|s| s.operation.starts_with("sources.refresh"))
        .map(|_| "permitted sources only, rate limits from manifests")
        .collect();
    Ok(
        json!({"name": def.name, "order": order, "steps": steps, "permissions": def.permissions, "network": sources, "note": "approval gates pause the run until a human decides; nothing is submitted to official sites"}),
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct DefinitionParam {
    /// Workflow definition (JSON object) …
    #[serde(default)]
    pub definition: Option<Value>,
    /// … or YAML/JSON text …
    #[serde(default)]
    pub text: Option<String>,
    /// … or a registered workflow name.
    #[serde(default)]
    pub name: Option<String>,
}

fn resolve_def(ctx: &Ctx<'_>, p: &DefinitionParam) -> OcResult<(WorkflowDef, Option<String>)> {
    if let Some(d) = &p.definition {
        return Ok((serde_json::from_value(d.clone())?, None));
    }
    if let Some(t) = &p.text {
        let def: WorkflowDef = serde_yaml::from_str(t).map_err(|e| OcError::invalid(format!("workflow text: {e}")))?;
        return Ok((def, None));
    }
    if let Some(n) = &p.name {
        let row = require(
            query_one(ctx.conn, "SELECT id, definition_json FROM workflows WHERE name = ?1", [n])?,
            "workflow",
            n,
        )?;
        return Ok((serde_json::from_value(row["definition"].clone())?, row["id"].as_str().map(str::to_string)));
    }
    Err(OcError::invalid("provide definition, text or name"))
}

fn op_list(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({"items": query_all(ctx.conn, "SELECT id, name, digest, created_by, created_at FROM workflows ORDER BY name", [])?}))
}

fn op_validate(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DefinitionParam = parse_params(p)?;
    let (def, _) = resolve_def(ctx, &p)?;
    match validate(&def) {
        Ok(order) => Ok(json!({"valid": true, "name": def.name, "order": order})),
        Err(e) => Ok(json!({"valid": false, "error": e})),
    }
}

fn op_register(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DefinitionParam = parse_params(p)?;
    let (def, _) = resolve_def(ctx, &p)?;
    validate(&def)?;
    let json = serde_json::to_value(&def)?;
    let digest = openclaim_security::digest::json_digest(&json);
    let id = openclaim_db::new_id();
    exec(
        ctx.conn,
        "INSERT INTO workflows(id, name, definition_json, digest, created_by, created_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(name) DO UPDATE SET definition_json = excluded.definition_json, digest = excluded.digest, created_by = excluded.created_by, created_at = excluded.created_at",
        (&id, &def.name, json.to_string(), &digest, ctx.actor_id(), &ctx.now),
    )?;
    let wid: String = crate::sql::scalar(ctx.conn, "SELECT id FROM workflows WHERE name = ?1", [&def.name])?.unwrap_or(id);
    Ok(json!({"workflow_id": wid, "name": def.name, "digest": digest}))
}

fn op_plan(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DefinitionParam = parse_params(p)?;
    let (def, _) = resolve_def(ctx, &p)?;
    plan(&def)
}

#[derive(Deserialize, JsonSchema)]
pub struct RunParams {
    pub name: String,
    /// Return immediately with the job id (always true; runs are durable).
    #[serde(default)]
    pub detach: bool,
}

fn op_run(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RunParams = parse_params(p)?;
    let row = require(
        query_one(ctx.conn, "SELECT id, definition_json FROM workflows WHERE name = ?1", [&p.name])?,
        "workflow",
        &p.name,
    )?;
    let def: WorkflowDef = serde_json::from_value(row["definition"].clone())?;
    validate(&def)?;
    for perm in &def.permissions {
        if !ctx.actor.has_scope(perm, None) {
            return Err(OcError::denied(format!("you lack '{perm}', which this workflow needs")).with_scopes(vec![perm.clone()]));
        }
    }
    let actor_snapshot = match &ctx.actor {
        Actor::Owner { .. } | Actor::System => Actor::System,
        other => other.clone(),
    };
    let run_id = openclaim_db::new_id();
    let job_id = crate::jobs::enqueue(ctx, "workflow_run", json!({"run_id": run_id}), None)?;
    let checkpoint = json!({"actor": actor_snapshot, "results": {}, "pending_approvals": {}, "skipped": []});
    exec(
        ctx.conn,
        "INSERT INTO workflow_runs(id, workflow_id, job_id, state, checkpoint_json, actor_id, created_at, updated_at) VALUES (?1,?2,?3,'queued',?4,?5,?6,?6)",
        (
            &run_id,
            row["id"].as_str().unwrap_or(""),
            &job_id,
            checkpoint.to_string(),
            ctx.actor_id(),
            &ctx.now,
        ),
    )?;
    Ok(json!({"run_id": run_id, "job_id": job_id, "status": "queued", "detached": true}))
}

#[derive(Deserialize, JsonSchema)]
pub struct RunIdParam {
    pub run_id: String,
}

fn load_run(ctx: &Ctx<'_>, run_id: &str) -> OcResult<Value> {
    let row = require(
        query_one(
            ctx.conn,
            "SELECT r.*, w.name, j.state AS job_state, j.progress, j.error_message FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id JOIN jobs j ON j.id = r.job_id WHERE r.id = ?1",
            [run_id],
        )?,
        "workflow run",
        run_id,
    )?;
    if !ctx.actor.is_owner() && row["actor_id"] != ctx.actor_id() {
        return Err(OcError::not_found(format!("workflow run '{run_id}' not found")));
    }
    Ok(row)
}

fn op_status(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RunIdParam = parse_params(p)?;
    let mut row = load_run(ctx, &p.run_id)?;
    if let Some(o) = row.as_object_mut()
        && let Some(cp) = o.get_mut("checkpoint")
        && let Some(c) = cp.as_object_mut()
    {
        c.remove("actor");
    }
    Ok(row)
}

fn op_resume(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RunIdParam = parse_params(p)?;
    let row = load_run(ctx, &p.run_id)?;
    let job_id = row["job_id"].as_str().unwrap_or("").to_string();
    match row["job_state"].as_str() {
        Some("paused_for_approval") | Some("failed") | Some("cancelled") => {
            exec(
                ctx.conn,
                "UPDATE jobs SET state = 'queued', cancel_requested = 0, next_run_at = ?2, error_code = NULL, error_message = NULL, finished_at = NULL, updated_at = ?2, lease_owner = NULL WHERE id = ?1",
                (&job_id, &ctx.now),
            )?;
            exec(
                ctx.conn,
                "UPDATE workflow_runs SET state = 'queued', updated_at = ?2 WHERE id = ?1",
                (&p.run_id, &ctx.now),
            )?;
            crate::jobs::job_event(ctx.conn, &job_id, "job.requeued", json!({"resume": true}), &ctx.now)?;
            ctx.jobs_enqueued = true;
            Ok(json!({"run_id": p.run_id, "job_id": job_id, "status": "queued"}))
        }
        other => Err(OcError::conflict(format!("run is {}", other.unwrap_or("unknown")))),
    }
}

fn op_cancel(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RunIdParam = parse_params(p)?;
    let row = load_run(ctx, &p.run_id)?;
    let job_id = row["job_id"].as_str().unwrap_or("").to_string();
    crate::modules::jobs_ops::ops()
        .iter()
        .find(|o| o.name == "jobs.cancel")
        .map(|o| (o.handler)(ctx, json!({"job_id": job_id})))
        .transpose()?;
    Ok(json!({"run_id": p.run_id, "status": "cancel_requested"}))
}

fn op_export(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        name: String,
    }
    let p: P = parse_params(p)?;
    let row = require(
        query_one(ctx.conn, "SELECT definition_json FROM workflows WHERE name = ?1", [&p.name])?,
        "workflow",
        &p.name,
    )?;
    let yaml = serde_yaml::to_string(&row["definition"]).map_err(|e| OcError::internal(e.to_string()))?;
    Ok(json!({"name": p.name, "definition": row["definition"], "yaml": yaml}))
}

/// Job body: runs steps in topological order from the checkpoint.
pub async fn execute_run(svc: &Service, job_id: &str, run_id: &str) -> OcResult<Value> {
    let (def, mut checkpoint) = {
        let guard = svc.inner.db.lock().map_err(|_| OcError::internal("db lock"))?;
        let row = require(
            query_one(
                &guard.conn,
                "SELECT r.checkpoint_json, w.definition_json FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id WHERE r.id = ?1",
                [run_id],
            )?,
            "workflow run",
            run_id,
        )?;
        let def: WorkflowDef = serde_json::from_value(row["definition"].clone())?;
        exec(
            &guard.conn,
            "UPDATE workflow_runs SET state = 'running', updated_at = ?2 WHERE id = ?1",
            (run_id, openclaim_domain::dates::now_rfc3339()),
        )?;
        (def, row["checkpoint"].clone())
    };
    let order = validate(&def)?;
    let actor: Actor = serde_json::from_value(checkpoint["actor"].clone()).unwrap_or(Actor::Anonymous);
    let mut results: BTreeMap<String, Value> = serde_json::from_value(checkpoint["results"].clone()).unwrap_or_default();
    let mut pending: BTreeMap<String, String> = serde_json::from_value(checkpoint["pending_approvals"].clone()).unwrap_or_default();
    let mut skipped: Vec<String> = serde_json::from_value(checkpoint["skipped"].clone()).unwrap_or_default();
    let total = order.len();
    for (i, step_id) in order.iter().enumerate() {
        if results.contains_key(step_id) || skipped.contains(step_id) {
            continue;
        }
        let step = def.steps.iter().find(|s| &s.id == step_id).unwrap();
        if let Some(cond) = &step.when {
            let actual = results.get(&cond.step).map(|r| lookup(r, &cond.path)).unwrap_or(Value::Null);
            if actual != cond.equals {
                skipped.push(step_id.clone());
                continue;
            }
        }
        let params = bind(&step.with, &results);
        let mut req = Request::new(&step.operation, params);
        req.non_interactive = true;
        req.require_approval = step.approval;
        req.transport = "workflow".into();
        req.approval_id = pending.get(step_id).cloned();
        let env = svc.execute_with_actor(req, Some(actor.clone()));
        let _ = svc.job_progress(job_id, ((i + 1) * 100 / total.max(1)) as u32, &format!("step {step_id}"));
        match env.error {
            None => {
                results.insert(step_id.clone(), env.data.unwrap_or(Value::Null));
                pending.remove(step_id);
                save_checkpoint(svc, run_id, &actor, &results, &pending, &skipped, "running")?;
            }
            Some(e) if e.code == ErrorCode::ApprovalRequired => {
                if let Some(a) = &e.approval_id {
                    pending.insert(step_id.clone(), a.clone());
                }
                save_checkpoint(svc, run_id, &actor, &results, &pending, &skipped, "paused_for_approval")?;
                return Err(e);
            }
            Some(e) => {
                save_checkpoint(svc, run_id, &actor, &results, &pending, &skipped, "failed")?;
                return Err(OcError::new(e.code, format!("step '{step_id}' failed: {}", e.message)).with_details(json!({"step": step_id, "error": e})));
            }
        }
        checkpoint = Value::Null;
    }
    let _ = checkpoint;
    save_checkpoint(svc, run_id, &actor, &results, &pending, &skipped, "succeeded")?;
    Ok(json!({"run_id": run_id, "completed": results.keys().collect::<Vec<_>>(), "skipped": skipped, "results": results}))
}

fn save_checkpoint(
    svc: &Service,
    run_id: &str,
    actor: &Actor,
    results: &BTreeMap<String, Value>,
    pending: &BTreeMap<String, String>,
    skipped: &[String],
    state: &str,
) -> OcResult<()> {
    let guard = svc.inner.db.lock().map_err(|_| OcError::internal("db lock"))?;
    let cp = json!({"actor": actor, "results": results, "pending_approvals": pending, "skipped": skipped});
    exec(
        &guard.conn,
        "UPDATE workflow_runs SET checkpoint_json = ?2, state = ?3, updated_at = ?4 WHERE id = ?1",
        (run_id, cp.to_string(), state, openclaim_domain::dates::now_rfc3339()),
    )?;
    Ok(())
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("workflows.list", "workflows.run", Read, "Registered workflows", super::Empty, op_list),
        op!(
            "workflows.validate",
            "workflows.run",
            Read,
            "Validate a workflow definition (JSON/YAML)",
            DefinitionParam,
            op_validate
        ),
        op!(
            "workflows.register",
            "workflows.run",
            Write,
            "Register or replace a workflow by name",
            DefinitionParam,
            op_register
        ),
        op!(
            "workflows.plan",
            "workflows.run",
            Read,
            "Preview order, scopes, network use and approval gates",
            DefinitionParam,
            op_plan
        ),
        op!(
            "workflows.run",
            "workflows.run",
            Write,
            "Start a durable run (returns job id)",
            RunParams,
            op_run
        ),
        op!(
            "workflows.resume",
            "workflows.run",
            Write,
            "Resume a paused or failed run from its checkpoint",
            RunIdParam,
            op_resume
        ),
        op!("workflows.status", "workflows.run", Read, "Run state and checkpoint", RunIdParam, op_status),
        op!("workflows.cancel", "workflows.run", Write, "Cancel a run", RunIdParam, op_cancel),
        op!("workflows.export", "workflows.run", Read, "Export a workflow as YAML", super::Empty, op_export),
    ]
}
