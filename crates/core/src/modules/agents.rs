//! Owner authentication, agent principals, approvals and audit (MDR §20).

use super::{IdParam, PageParams, Pager, parse_params};
use crate::ctx::{Actor, Ctx};
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require, scalar};
use openclaim_domain::scopes::{ALL_SCOPES, OWNER_ONLY_SCOPES, is_known_scope};
use openclaim_domain::{OcError, OcResult};
use openclaim_security::tokens::{hash_passphrase, hash_token, new_token, verify_passphrase};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

// ------------------------------------------------------------------ owner

#[derive(Deserialize, JsonSchema)]
pub struct PassphraseParam {
    pub passphrase: String,
}

fn owner_exists(conn: &openclaim_db::rusqlite::Connection) -> OcResult<bool> {
    Ok(scalar::<i64, _>(conn, "SELECT count(*) FROM owner", [])?.unwrap_or(0) > 0)
}

fn op_owner_init(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PassphraseParam = parse_params(p)?;
    if owner_exists(ctx.conn)? {
        return Err(OcError::conflict("an owner passphrase is already set; use owner.set_passphrase"));
    }
    if ctx.actor.is_agent() {
        return Err(OcError::denied("agents cannot initialize the owner"));
    }
    if p.passphrase.chars().count() < 8 {
        return Err(OcError::invalid("passphrase must be at least 8 characters"));
    }
    let hash = hash_passphrase(&p.passphrase).map_err(OcError::internal)?;
    exec(ctx.conn, "INSERT INTO owner(id, passphrase_hash, created_at, updated_at) VALUES ('owner', ?1, ?2, ?2)", (&hash, &ctx.now))?;
    ctx.audit("owner.init", "owner", json!({}))?;
    Ok(json!({"initialized": true, "note": "keep this passphrase; it gates approvals, personal data and the GUI"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct UnlockParams {
    pub passphrase: String,
    #[serde(default)]
    pub ttl_minutes: Option<u64>,
}

fn op_owner_unlock(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: UnlockParams = parse_params(p)?;
    if ctx.actor.is_agent() {
        return Err(OcError::denied("agents cannot unlock an owner session"));
    }
    {
        let mut attempts = ctx.svc.inner.unlock_attempts.lock().map_err(|_| OcError::internal("lock"))?;
        let now = time::OffsetDateTime::now_utc();
        attempts.retain(|t| now - *t < time::Duration::minutes(1));
        if attempts.len() >= 5 {
            return Err(OcError::new(openclaim_domain::ErrorCode::Unavailable, "too many unlock attempts; wait a minute"));
        }
        attempts.push(now);
    }
    let hash: Option<String> = scalar(ctx.conn, "SELECT passphrase_hash FROM owner WHERE id = 'owner'", [])?;
    let Some(hash) = hash else {
        return Err(OcError::conflict("no owner passphrase set").with_next_action("run `openclaim init`"));
    };
    if !verify_passphrase(&p.passphrase, &hash) {
        ctx.audit("owner.unlock_failed", "owner", json!({"transport": ctx.transport}))?;
        return Err(OcError::denied("passphrase does not match"));
    }
    let ttl = p.ttl_minutes.unwrap_or_else(|| crate::modules::config::get_u64(ctx.conn, "session.ttl_minutes", 60)).clamp(1, 1440);
    let token = new_token("ocs");
    let expires = time::OffsetDateTime::now_utc() + time::Duration::minutes(ttl as i64);
    ctx.svc.inner.sessions.lock().map_err(|_| OcError::internal("lock"))?.insert(token.clone(), crate::OwnerSession { id: token.clone(), expires_at: expires, transport: ctx.transport.clone() });
    ctx.audit("owner.unlock", "owner", json!({"transport": ctx.transport, "ttl_minutes": ttl}))?;
    Ok(json!({"session": token, "expires_at": openclaim_domain::dates::format_ts(expires), "note": "export OPENCLAIM_SESSION in your shell; sessions live only in daemon memory"}))
}

fn op_owner_lock(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    if let Actor::Owner { session_id } = &ctx.actor {
        ctx.svc.inner.sessions.lock().map_err(|_| OcError::internal("lock"))?.remove(session_id);
    }
    Ok(json!({"locked": true}))
}

#[derive(Deserialize, JsonSchema)]
pub struct SetPassphraseParams {
    pub current: String,
    pub new: String,
}

fn op_owner_set_passphrase(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SetPassphraseParams = parse_params(p)?;
    let hash: String = require(scalar(ctx.conn, "SELECT passphrase_hash FROM owner WHERE id = 'owner'", [])?, "owner", "owner")?;
    if !verify_passphrase(&p.current, &hash) {
        return Err(OcError::denied("current passphrase does not match"));
    }
    if p.new.chars().count() < 8 {
        return Err(OcError::invalid("new passphrase must be at least 8 characters"));
    }
    exec(ctx.conn, "UPDATE owner SET passphrase_hash = ?1, updated_at = ?2 WHERE id = 'owner'", (hash_passphrase(&p.new).map_err(OcError::internal)?, &ctx.now))?;
    ctx.svc.inner.sessions.lock().map_err(|_| OcError::internal("lock"))?.retain(|_, s| s.transport == "system");
    Ok(json!({"changed": true, "note": "all owner sessions were ended"}))
}

fn op_owner_status(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({"initialized": owner_exists(ctx.conn)?, "actor": ctx.actor, "unlocked": ctx.actor.is_owner()}))
}

// ------------------------------------------------------------------ agents

#[derive(Deserialize, JsonSchema)]
pub struct RegisterParams {
    pub name: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub expires_in_days: Option<u32>,
    /// agent | extension
    #[serde(default)]
    pub kind: Option<String>,
    /// For extensions: the allowed origin (chrome-extension://id).
    #[serde(default)]
    pub allowed_origin: Option<String>,
}

fn check_scopes(scopes: &[String]) -> OcResult<()> {
    for s in scopes {
        if !is_known_scope(s) {
            return Err(OcError::invalid(format!("unknown scope '{s}'")));
        }
        let base = s.split(':').next().unwrap_or(s);
        if OWNER_ONLY_SCOPES.contains(&base) {
            return Err(OcError::policy(format!("'{s}' can never be granted to an agent")));
        }
    }
    Ok(())
}

fn op_register(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RegisterParams = parse_params(p)?;
    if p.name.trim().is_empty() || p.name.len() > 60 {
        return Err(OcError::invalid("name must be 1-60 characters"));
    }
    check_scopes(&p.scopes)?;
    let kind = p.kind.as_deref().unwrap_or("agent");
    if !matches!(kind, "agent" | "extension") {
        return Err(OcError::invalid("kind must be agent or extension"));
    }
    let id = openclaim_db::new_id();
    let token = new_token(if kind == "extension" { "ocx" } else { "oca" });
    let expires = p.expires_in_days.map(|d| openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::days(d as i64)));
    exec(
        ctx.conn,
        "INSERT INTO agent_principals(id, display_name, kind, token_hash, executor_fingerprint, scopes_json, allowed_origin, created_at, expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        (&id, p.name.trim(), kind, hash_token(&token), &ctx.transport, serde_json::to_string(&p.scopes)?, &p.allowed_origin, &ctx.now, &expires),
    )?;
    ctx.audit("agents.register", &id, json!({"name": p.name, "scopes": p.scopes, "kind": kind}))?;
    Ok(json!({"agent_id": id, "name": p.name.trim(), "kind": kind, "scopes": p.scopes, "expires_at": expires, "token": if ctx.dry_run { Value::Null } else { json!(token) }, "note": "the token is shown once; give it to the agent as OPENCLAIM_AGENT_TOKEN"}))
}

fn op_list(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({"items": query_all(ctx.conn, "SELECT id, display_name, kind, scopes_json, allowed_origin, created_at, expires_at, revoked_at FROM agent_principals ORDER BY created_at", [])?}))
}

fn op_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT id, display_name, kind, scopes_json, allowed_origin, executor_fingerprint, created_at, expires_at, revoked_at FROM agent_principals WHERE id = ?1", [&p.id])?, "agent", &p.id)?;
    let sessions = query_all(ctx.conn, "SELECT id, transport, started_at, last_seen FROM agent_sessions WHERE principal_id = ?1 ORDER BY last_seen DESC LIMIT 20", [&p.id])?;
    let pending = query_all(ctx.conn, "SELECT id, op, decision, created_at, expires_at FROM agent_approvals WHERE principal_id = ?1 AND decision = 'pending'", [&p.id])?;
    Ok(json!({"agent": row, "sessions": sessions, "pending_approvals": pending}))
}

#[derive(Deserialize, JsonSchema)]
pub struct AuthorizeParams {
    pub id: String,
    #[serde(default)]
    pub add_scopes: Vec<String>,
    #[serde(default)]
    pub remove_scopes: Vec<String>,
}

fn op_authorize(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: AuthorizeParams = parse_params(p)?;
    check_scopes(&p.add_scopes)?;
    let row = require(query_one(ctx.conn, "SELECT scopes_json, revoked_at FROM agent_principals WHERE id = ?1", [&p.id])?, "agent", &p.id)?;
    if !row["revoked_at"].is_null() {
        return Err(OcError::conflict("agent is revoked"));
    }
    let mut scopes: Vec<String> = serde_json::from_value(row["scopes"].clone()).unwrap_or_default();
    scopes.retain(|s| !p.remove_scopes.contains(s));
    for s in p.add_scopes {
        if !scopes.contains(&s) {
            scopes.push(s);
        }
    }
    exec(ctx.conn, "UPDATE agent_principals SET scopes_json = ?2 WHERE id = ?1", (&p.id, serde_json::to_string(&scopes)?))?;
    ctx.audit("agents.authorize", &p.id, json!({"scopes": scopes}))?;
    Ok(json!({"agent_id": p.id, "scopes": scopes}))
}

fn op_scopes(_ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({"scopes": ALL_SCOPES.iter().map(|(s, d)| json!({"scope": s, "description": d, "owner_only": OWNER_ONLY_SCOPES.contains(s)})).collect::<Vec<_>>(), "note": "no scope allows official claim submission"}))
}

fn op_sessions(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let agents = query_all(ctx.conn, "SELECT s.id, s.principal_id, p.display_name, s.transport, s.started_at, s.last_seen FROM agent_sessions s JOIN agent_principals p ON p.id = s.principal_id WHERE p.revoked_at IS NULL ORDER BY s.last_seen DESC LIMIT 100", [])?;
    let owner: Vec<Value> = ctx.svc.inner.sessions.lock().map_err(|_| OcError::internal("lock"))?.values().filter(|s| s.transport != "system").map(|s| json!({"transport": s.transport, "expires_at": openclaim_domain::dates::format_ts(s.expires_at)})).collect();
    Ok(json!({"agents": agents, "owner_sessions": owner}))
}

fn op_revoke(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let n = exec(ctx.conn, "UPDATE agent_principals SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL", (&p.id, &ctx.now))?;
    if n == 0 {
        return Err(OcError::not_found("agent not found or already revoked"));
    }
    exec(ctx.conn, "UPDATE agent_approvals SET decision = 'invalidated', decided_at = ?2 WHERE principal_id = ?1 AND decision IN ('pending','approved')", (&p.id, &ctx.now))?;
    let cancelled = exec(ctx.conn, "UPDATE jobs SET state = 'cancelled', cancel_requested = 1, finished_at = ?2, updated_at = ?2 WHERE owner_actor_id = ?1 AND state IN ('queued','paused_for_approval')", (format!("agent:{}", p.id), &ctx.now))?;
    exec(ctx.conn, "UPDATE jobs SET cancel_requested = 1 WHERE owner_actor_id = ?1 AND state = 'running'", [format!("agent:{}", p.id)])?;
    ctx.audit("agents.revoke", &p.id, json!({"jobs_cancelled": cancelled}))?;
    ctx.emit("agent.revoked", json!({"agent_id": p.id}))?;
    Ok(json!({"agent_id": p.id, "revoked": true, "jobs_cancelled": cancelled}))
}

fn op_audit(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let actor = format!("agent:{}", p.id);
    Ok(json!({
        "agent_id": p.id,
        "commands": query_all(ctx.conn, "SELECT command, args_redacted_json, result, dry_run, created_at FROM command_runs WHERE actor_id = ?1 ORDER BY created_at DESC LIMIT 500", [&actor])?,
        "disclosures": query_all(ctx.conn, "SELECT subject_kind, subject_id, fields, at FROM data_disclosures WHERE principal_id = ?1 ORDER BY at DESC LIMIT 500", [&p.id])?,
        "approvals": query_all(ctx.conn, "SELECT id, op, decision, created_at, decided_at, consumed_at FROM agent_approvals WHERE principal_id = ?1 ORDER BY created_at DESC LIMIT 200", [&p.id])?,
        "proposals": query_all(ctx.conn, "SELECT id, person_id, validation_state, created_at FROM proposed_facts WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 200", [&actor])?,
    }))
}

// --------------------------------------------------------------- approvals

fn op_approvals_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        state: Option<String>,
        #[serde(flatten)]
        page: PageParams,
    }
    let p: P = parse_params(p)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let mine = ctx.actor.principal_id().map(str::to_string);
    let rows = query_all(
        ctx.conn,
        &format!("SELECT a.id, a.principal_id, p.display_name, a.op, a.scope, a.decision, a.preview_json, a.created_at, a.expires_at, a.decided_at FROM agent_approvals a JOIN agent_principals p ON p.id = a.principal_id
                  WHERE a.created_at <= ?1 AND (?2 IS NULL OR a.decision = ?2) AND (?3 IS NULL OR a.principal_id = ?3) ORDER BY a.created_at DESC{}", pager.sql_limit()),
        (&pager.snapshot, &p.state, &mine),
    )?;
    Ok(pager.wrap(rows, None))
}

fn op_approvals_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT a.*, p.display_name FROM agent_approvals a JOIN agent_principals p ON p.id = a.principal_id WHERE a.id = ?1", [&p.id])?, "approval", &p.id)?;
    if let Some(pid) = ctx.actor.principal_id() {
        if row["principal_id"] != pid {
            return Err(OcError::not_found("approval not found"));
        }
    }
    let mut row = row;
    if !ctx.actor.is_owner() {
        if let Some(o) = row.as_object_mut() {
            o.remove("params");
            o.remove("binding");
        }
    }
    Ok(row)
}

#[derive(Deserialize, JsonSchema)]
pub struct RequestApprovalParams {
    pub op: String,
    #[serde(default)]
    pub params: Value,
}

fn op_approvals_request(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: RequestApprovalParams = parse_params(p)?;
    let Some(pid) = ctx.actor.principal_id().map(str::to_string) else {
        return Err(OcError::denied("only registered agents request approvals; owners act directly"));
    };
    let op = crate::ops::find(&p.op).ok_or_else(|| OcError::invalid(format!("unknown operation '{}'", p.op)))?;
    if op.effect == openclaim_domain::scopes::Effect::OwnerOnly || OWNER_ONLY_SCOPES.contains(&op.scope) {
        return Err(OcError::denied(format!("'{}' can never be approved for an agent", p.op)));
    }
    let binding = crate::authz::compute_binding(ctx.conn, &p.params)?;
    let digest = crate::authz::action_digest(op.name, &p.params, &pid, &binding);
    if let Some(existing) = scalar::<String, _>(ctx.conn, "SELECT id FROM agent_approvals WHERE action_digest = ?1 AND principal_id = ?2 AND decision = 'pending' AND expires_at > ?3", (&digest, &pid, &ctx.now))? {
        return Ok(json!({"approval_id": existing, "decision": "pending", "existing": true}));
    }
    let id = openclaim_db::new_id();
    let expires = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::minutes(crate::authz::APPROVAL_TTL_MINUTES));
    let preview = json!({"op": op.name, "summary": op.summary, "effect": op.effect, "params": openclaim_security::redact::redact_json(&p.params)});
    exec(ctx.conn, "INSERT INTO agent_approvals(id, principal_id, op, action_digest, scope, params_json, binding_json, preview_json, created_at, expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", (&id, &pid, op.name, &digest, op.scope, p.params.to_string(), binding.to_string(), preview.to_string(), &ctx.now, &expires))?;
    ctx.emit("approval.requested", json!({"approval_id": id, "principal_id": pid, "op": op.name}))?;
    Ok(json!({"approval_id": id, "decision": "pending", "expires_at": expires}))
}

#[derive(Deserialize, JsonSchema)]
pub struct DecideParams {
    pub id: String,
    #[serde(default)]
    pub note: Option<String>,
}

fn decide(ctx: &mut Ctx<'_>, id: &str, approve: bool) -> OcResult<Value> {
    let row = require(query_one(ctx.conn, "SELECT id, decision, expires_at, op, action_digest, params_json, principal_id FROM agent_approvals WHERE id = ?1", [id])?, "approval", id)?;
    if row["decision"] != "pending" {
        return Err(OcError::conflict(format!("approval is already {}", row["decision"])));
    }
    if row["expires_at"].as_str().unwrap_or("") < ctx.now.as_str() {
        exec(ctx.conn, "UPDATE agent_approvals SET decision = 'expired' WHERE id = ?1", [id])?;
        return Err(OcError::conflict("approval expired; the agent must request again"));
    }
    if approve {
        // The digest must still describe the current records.
        let params: Value = row["params"].clone();
        let binding = crate::authz::compute_binding(ctx.conn, &params)?;
        let digest = crate::authz::action_digest(row["op"].as_str().unwrap_or(""), &params, row["principal_id"].as_str().unwrap_or(""), &binding);
        if row["action_digest"].as_str() != Some(&digest) {
            exec(ctx.conn, "UPDATE agent_approvals SET decision = 'invalidated', decided_at = ?2 WHERE id = ?1", (id, &ctx.now))?;
            return Err(OcError::conflict("the records this action refers to changed since the request; it was invalidated"));
        }
    }
    let decision = if approve { "approved" } else { "denied" };
    exec(ctx.conn, "UPDATE agent_approvals SET decision = ?2, decided_by = ?3, decided_at = ?4 WHERE id = ?1", (id, decision, ctx.actor_id(), &ctx.now))?;
    ctx.audit(if approve { "approvals.approve" } else { "approvals.deny" }, id, json!({"op": row["op"], "principal_id": row["principal_id"]}))?;
    ctx.emit("approval.decided", json!({"approval_id": id, "decision": decision}))?;
    if approve {
        // Paused workflow jobs owned by that principal can continue.
        let n = exec(ctx.conn, "UPDATE jobs SET state = 'queued', next_run_at = ?2, updated_at = ?2, error_code = NULL, error_message = NULL WHERE state = 'paused_for_approval' AND owner_actor_id = ?1", (format!("agent:{}", row["principal_id"].as_str().unwrap_or("")), &ctx.now))?;
        if n > 0 {
            ctx.jobs_enqueued = true;
        }
    }
    Ok(json!({"approval_id": id, "decision": decision}))
}

fn op_approve(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DecideParams = parse_params(p)?;
    decide(ctx, &p.id, true)
}

fn op_deny(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DecideParams = parse_params(p)?;
    decide(ctx, &p.id, false)
}

fn op_expire(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let n = exec(ctx.conn, "UPDATE agent_approvals SET decision = 'expired' WHERE decision = 'pending' AND expires_at <= ?1", [&ctx.now])?;
    Ok(json!({"expired": n}))
}

// ------------------------------------------------------------------ audit

fn op_audit_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PageParams = parse_params(p)?;
    let pager = Pager::from(&p, &ctx.now)?;
    let rows = query_all(ctx.conn, &format!("SELECT seq, id, actor, action, timestamp, redacted_subject, details_json, integrity_hash FROM audit_log WHERE timestamp <= ?1 ORDER BY seq DESC{}", pager.sql_limit()), [&pager.snapshot])?;
    Ok(pager.wrap(rows, None))
}

fn op_audit_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    require(query_one(ctx.conn, "SELECT * FROM audit_log WHERE id = ?1", [&p.id])?, "audit entry", &p.id)
}

fn op_audit_verify(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let (count, broken) = crate::audit::verify(ctx.conn)?;
    Ok(json!({"entries": count, "intact": broken.is_none(), "first_broken_seq": broken}))
}

fn op_audit_export(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        path: Option<String>,
    }
    let p: P = parse_params(p)?;
    let rows = query_all(ctx.conn, "SELECT seq, id, actor, action, timestamp, redacted_subject, details_json, prev_hash, integrity_hash FROM audit_log ORDER BY seq", [])?;
    let runs = query_all(ctx.conn, "SELECT actor_id, command, args_redacted_json, result, dry_run, created_at FROM command_runs ORDER BY created_at", [])?;
    let disclosures = query_all(ctx.conn, "SELECT principal_id, subject_kind, subject_id, fields, at FROM data_disclosures ORDER BY at", [])?;
    let export = json!({"exported_at": ctx.now, "audit_log": rows, "command_runs": runs, "disclosures": disclosures});
    if let Some(path) = &p.path {
        if !ctx.dry_run {
            std::fs::write(path, serde_json::to_string_pretty(&export)?).map_err(|e| OcError::invalid(format!("write {path}: {e}")))?;
        }
        return Ok(json!({"path": path, "entries": export["audit_log"].as_array().map(|a| a.len()).unwrap_or(0)}));
    }
    Ok(export)
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("owner.init", "schema.read", Write, "Set the owner passphrase on first run", PassphraseParam, op_owner_init),
        op!("owner.unlock", "schema.read", Write, "Prove the passphrase; returns a memory-only session", UnlockParams, op_owner_unlock),
        op!("owner.lock", "schema.read", Write, "End the current owner session", super::Empty, op_owner_lock),
        op!("owner.set_passphrase", "agents.manage", OwnerOnly, "Change the owner passphrase", SetPassphraseParams, op_owner_set_passphrase),
        op!("owner.status", "schema.read", Read, "Who the daemon thinks you are", super::Empty, op_owner_status),
        op!("agents.register", "agents.manage", OwnerOnly, "Register a named agent or extension principal; shows the token once", RegisterParams, op_register),
        op!("agents.list", "agents.manage", OwnerOnly, "List principals", super::Empty, op_list),
        op!("agents.show", "agents.manage", OwnerOnly, "Principal details, sessions and pending approvals", IdParam, op_show),
        op!("agents.authorize", "agents.manage", OwnerOnly, "Add or remove scopes", AuthorizeParams, op_authorize),
        op!("agents.scopes", "schema.read", Read, "All capability scopes", super::Empty, op_scopes),
        op!("agents.sessions", "agents.manage", OwnerOnly, "Active agent and owner sessions", super::Empty, op_sessions),
        op!("agents.revoke", "agents.manage", OwnerOnly, "Revoke a principal and cancel its queued work", IdParam, op_revoke),
        op!("agents.audit", "agents.manage", OwnerOnly, "What an agent ran, read and proposed", IdParam, op_audit),
        op!("approvals.list", "jobs.read", Read, "Pending and decided approvals (agents see their own)", PageParams, op_approvals_list),
        op!("approvals.show", "jobs.read", Read, "Approval preview and state", IdParam, op_approvals_show, mcp = "get_approval_status"),
        op!("approvals.request", "jobs.read", Write, "Agent: request a human decision for a named action", RequestApprovalParams, op_approvals_request, mcp = "request_approval"),
        op!("approvals.approve", "approvals.decide", OwnerOnly, "Approve the exact action (digest-bound)", DecideParams, op_approve),
        op!("approvals.deny", "approvals.decide", OwnerOnly, "Deny an approval", DecideParams, op_deny),
        op!("approvals.expire", "approvals.decide", OwnerOnly, "Expire overdue approvals now", super::Empty, op_expire),
        op!("audit.list", "audit.read", Read, "Audit trail (newest first)", PageParams, op_audit_list),
        op!("audit.show", "audit.read", Read, "One audit entry", IdParam, op_audit_show),
        op!("audit.verify", "audit.read", Read, "Verify the audit hash chain", super::Empty, op_audit_verify),
        op!("audit.export", "audit.read", Read, "Export audit log, command runs and disclosures", super::Empty, op_audit_export),
    ]
}
