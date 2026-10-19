//! Actor resolution, scope checks and the approval transaction lifecycle (MDR §20).

use crate::ctx::{Actor, Credentials, Request};
use crate::ops::OpSpec;
use crate::sql::{exec, query_one, scalar};
use crate::{Service, sql};
use openclaim_db::rusqlite::Connection;
use openclaim_domain::model::ApprovalDecision;
use openclaim_domain::scopes::Effect;
use openclaim_domain::{ErrorCode, OcError, OcResult};
use openclaim_security::digest::json_digest;
use openclaim_security::tokens::hash_token;
use serde_json::{Value, json};

pub const APPROVAL_TTL_MINUTES: i64 = 60;

pub fn resolve_actor(svc: &Service, conn: &Connection, creds: &Credentials, now: &str, transport: &str) -> OcResult<Actor> {
    if let Some(token) = creds.agent_token.as_deref().filter(|t| !t.is_empty()) {
        let row = query_one(
            conn,
            "SELECT id, display_name, scopes_json, kind, expires_at, revoked_at FROM agent_principals WHERE token_hash = ?1",
            [hash_token(token)],
        )?;
        let Some(row) = row else {
            return Err(OcError::denied("agent token is not recognized").with_next_action("ask the owner to run `openclaim agents register`"));
        };
        if !row["revoked_at"].is_null() {
            return Err(OcError::denied("this agent principal was revoked"));
        }
        if let Some(exp) = row["expires_at"].as_str() {
            if exp < now {
                return Err(OcError::denied("this agent principal has expired"));
            }
        }
        let id = row["id"].as_str().unwrap_or_default().to_string();
        exec(
            conn,
            "INSERT INTO agent_sessions(id, principal_id, transport, started_at, last_seen) VALUES (?1,?2,?3,?4,?4)
             ON CONFLICT(id) DO UPDATE SET last_seen = excluded.last_seen",
            (format!("{id}:{transport}"), &id, transport, now),
        )?;
        let scopes: Vec<String> = serde_json::from_value(row["scopes"].clone()).unwrap_or_default();
        return Ok(Actor::Agent { id, name: row["display_name"].as_str().unwrap_or_default().to_string(), scopes, principal_kind: row["kind"].as_str().unwrap_or("agent").to_string() });
    }
    if let Some(session) = creds.session.as_deref().filter(|s| !s.is_empty()) {
        let sessions = svc.inner.sessions.lock().map_err(|_| OcError::internal("session lock poisoned"))?;
        return match sessions.get(session) {
            Some(s) if s.expires_at > time::OffsetDateTime::now_utc() => Ok(Actor::Owner { session_id: s.id.clone() }),
            Some(_) => Err(OcError::denied("owner session expired").with_next_action("run `openclaim unlock` again")),
            None => Err(OcError::denied("owner session is not valid for this daemon").with_next_action("run `openclaim unlock`")),
        };
    }
    Ok(Actor::Anonymous)
}

/// Re-validates a stored actor snapshot (revocation/expiry since the run started).
pub fn recheck_actor(conn: &Connection, actor: Actor, now: &str) -> OcResult<Actor> {
    if let Actor::Agent { id, .. } = &actor {
        let row = query_one(conn, "SELECT scopes_json, display_name, kind, revoked_at, expires_at FROM agent_principals WHERE id = ?1", [id])?;
        let Some(row) = row else { return Err(OcError::denied("agent principal no longer exists")) };
        if !row["revoked_at"].is_null() {
            return Err(OcError::denied("agent principal was revoked"));
        }
        if row["expires_at"].as_str().is_some_and(|e| e < now) {
            return Err(OcError::denied("agent principal expired"));
        }
        let scopes: Vec<String> = serde_json::from_value(row["scopes"].clone()).unwrap_or_default();
        return Ok(Actor::Agent { id: id.clone(), name: row["display_name"].as_str().unwrap_or("").into(), scopes, principal_kind: row["kind"].as_str().unwrap_or("agent").into() });
    }
    Ok(actor)
}

pub struct Authorized {
    pub consumed_approval: Option<String>,
}

/// Decides whether `actor` may run `op` with `params` now.
pub fn authorize(conn: &Connection, actor: &Actor, op: &OpSpec, params: &Value, req: &Request, now: &str) -> OcResult<Authorized> {
    let resource = op.resource_param.and_then(|p| params.get(p)).and_then(|v| v.as_str());
    match actor {
        Actor::System | Actor::Owner { .. } => Ok(Authorized { consumed_approval: None }),
        Actor::Anonymous => {
            if actor.has_scope(op.scope, resource) {
                Ok(Authorized { consumed_approval: None })
            } else {
                Err(OcError::denied(format!("'{}' needs an unlocked owner session or an agent token with scope '{}'", op.name, op.scope))
                    .with_scopes(vec![op.scope.to_string()])
                    .with_next_action("run `openclaim unlock` (human) or set OPENCLAIM_AGENT_TOKEN (registered agent)"))
            }
        }
        Actor::Agent { id, principal_kind, .. } => {
            if op.effect == Effect::OwnerOnly || openclaim_domain::scopes::OWNER_ONLY_SCOPES.contains(&op.scope) {
                return Err(OcError::denied(format!("'{}' can only be performed by the owner in an authenticated session", op.name)));
            }
            if principal_kind == "extension" && !op.scope.starts_with("extension.") && op.effect != Effect::Read {
                return Err(OcError::denied("browser extension principals may only read and use prefill packages"));
            }
            let has_scope = actor.has_scope(op.scope, resource);
            let effect = if req.require_approval && op.effect == Effect::Write { Effect::Elevated } else { op.effect };
            match effect {
                Effect::Read => {
                    if has_scope {
                        Ok(Authorized { consumed_approval: None })
                    } else {
                        Err(OcError::denied(format!("agent lacks scope '{}'", op.scope)).with_scopes(vec![op.scope.to_string()]))
                    }
                }
                Effect::Write if has_scope => Ok(Authorized { consumed_approval: None }),
                Effect::Write | Effect::Elevated => {
                    // Needs a human decision bound to this exact action.
                    let binding = compute_binding(conn, params)?;
                    let digest = action_digest(op.name, params, id, &binding);
                    if let Some(approval_id) = &req.approval_id {
                        let consumed = consume_approval(conn, approval_id, id, &digest, now)?;
                        return Ok(Authorized { consumed_approval: Some(consumed) });
                    }
                    if let Some(existing) = query_one(conn, "SELECT id, decision FROM agent_approvals WHERE action_digest = ?1 AND principal_id = ?2 AND decision IN ('pending','approved') AND expires_at > ?3 ORDER BY created_at DESC LIMIT 1", (&digest, id, now))? {
                        let aid = existing["id"].as_str().unwrap_or_default().to_string();
                        if existing["decision"] == "approved" {
                            let consumed = consume_approval(conn, &aid, id, &digest, now)?;
                            return Ok(Authorized { consumed_approval: Some(consumed) });
                        }
                        return Err(approval_required(&aid, op, has_scope));
                    }
                    let aid = create_approval(conn, id, op, params, &binding, &digest, now)?;
                    Err(approval_required(&aid, op, has_scope))
                }
                Effect::OwnerOnly => unreachable!("handled above"),
            }
        }
    }
}

fn approval_required(approval_id: &str, op: &OpSpec, has_scope: bool) -> OcError {
    let mut e = OcError::new(ErrorCode::ApprovalRequired, "An explicitly authorized user must approve this action.");
    e.approval_id = Some(approval_id.to_string());
    e.needed_scopes = if has_scope { vec![] } else { vec![op.scope.to_string()] };
    e.next_action = Some(format!("ask the owner to review `openclaim approvals show {approval_id}`; then re-run with --approval {approval_id}"));
    e
}

/// Versions of every record the action refers to, so later edits invalidate the approval.
pub fn compute_binding(conn: &Connection, params: &Value) -> OcResult<Value> {
    let mut b = serde_json::Map::new();
    let lookups: &[(&str, &str, &str)] = &[
        ("claim_id", "claims", "version"),
        ("person_id", "people", "version"),
        ("opportunity_id", "opportunities", "version"),
        ("document_id", "documents", "version"),
        ("source_id", "sources", "version"),
    ];
    // A bare `id` names a record in exactly one of these tables; bind whichever matches.
    let generic: Vec<(&str, &str, &str)> = params
        .get("id")
        .and_then(|v| v.as_str())
        .map(|_| lookups.iter().map(|(_, t, c)| ("id", *t, *c)).collect())
        .unwrap_or_default();
    for (param, table, col) in lookups.iter().chain(generic.iter()) {
        if let Some(id) = params.get(*param).and_then(|v| v.as_str()) {
            let v: Option<i64> = scalar(conn, &format!("SELECT {col} FROM {table} WHERE id = ?1"), [id])?;
            if v.is_none() && *param == "id" {
                continue;
            }
            b.insert(format!("{table}:{id}"), json!(v));
            if *table == "opportunities" {
                let rv: Option<i64> = scalar(conn, "SELECT COALESCE(max(version), 0) FROM opportunity_rules WHERE opportunity_id = ?1", [id])?;
                b.insert(format!("rules:{id}"), json!(rv));
            }
        }
    }
    Ok(Value::Object(b))
}

pub fn action_digest(op: &str, params: &Value, principal_id: &str, binding: &Value) -> String {
    json_digest(&json!({"op": op, "params": params, "principal": principal_id, "binding": binding}))
}

fn create_approval(conn: &Connection, principal_id: &str, op: &OpSpec, params: &Value, binding: &Value, digest: &str, now: &str) -> OcResult<String> {
    let id = openclaim_db::new_id();
    let expires = time::OffsetDateTime::now_utc() + time::Duration::minutes(APPROVAL_TTL_MINUTES);
    let preview = json!({
        "op": op.name,
        "summary": op.summary,
        "effect": op.effect,
        "params": openclaim_security::redact::redact_json(params),
        "side_effects": if op.network { "local records and a network fetch job" } else { "local records only" },
    });
    exec(
        conn,
        "INSERT INTO agent_approvals(id, principal_id, op, action_digest, scope, params_json, binding_json, preview_json, created_at, expires_at, decision)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending')",
        (&id, principal_id, op.name, digest, op.scope, params.to_string(), binding.to_string(), preview.to_string(), now, openclaim_domain::dates::format_ts(expires)),
    )?;
    crate::events::emit(conn, "approval.requested", json!({"approval_id": id, "principal_id": principal_id, "op": op.name}))?;
    crate::audit::append(conn, &format!("agent:{principal_id}"), "approval.request", op.name, json!({"approval_id": id}))?;
    Ok(id)
}

fn consume_approval(conn: &Connection, approval_id: &str, principal_id: &str, digest: &str, now: &str) -> OcResult<String> {
    let row = sql::require(
        query_one(conn, "SELECT id, principal_id, action_digest, decision, expires_at FROM agent_approvals WHERE id = ?1", [approval_id])?,
        "approval",
        approval_id,
    )?;
    if row["principal_id"].as_str() != Some(principal_id) {
        return Err(OcError::denied("this approval belongs to a different principal"));
    }
    let decision = row["decision"].as_str().unwrap_or("");
    if decision == "pending" {
        let mut e = OcError::new(ErrorCode::ApprovalRequired, "approval is still pending a human decision");
        e.approval_id = Some(approval_id.to_string());
        return Err(e);
    }
    if decision != "approved" {
        return Err(OcError::denied(format!("approval {approval_id} is {decision}")));
    }
    if row["expires_at"].as_str().unwrap_or("") < now {
        exec(conn, "UPDATE agent_approvals SET decision = 'expired' WHERE id = ?1", [approval_id])?;
        return Err(OcError::denied("approval expired before it was used").with_next_action("request a new approval"));
    }
    if row["action_digest"].as_str() != Some(digest) {
        exec(conn, "UPDATE agent_approvals SET decision = 'invalidated', decided_at = ?2 WHERE id = ?1", (approval_id, now))?;
        return Err(OcError::conflict("approval invalidated: the parameters or the records it was bound to have changed").with_next_action("request a new approval"));
    }
    exec(conn, "UPDATE agent_approvals SET decision = ?2, consumed_at = ?3 WHERE id = ?1", (approval_id, ApprovalDecision::Consumed.as_str(), now))?;
    Ok(approval_id.to_string())
}
