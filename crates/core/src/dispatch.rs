//! Request execution: one transaction per command.

use crate::ctx::{Actor, Ctx, Request};
use crate::sql::{exec, query_one};
use crate::{Service, authz, ops};
use openclaim_domain::envelope::Pagination;
use openclaim_domain::scopes::Effect;
use openclaim_domain::{Envelope, ErrorCode, OcError, OcResult};
use openclaim_security::digest::json_digest;
use openclaim_security::redact::redact_json;
use serde_json::{Value, json};
use std::time::Instant;

pub type Response = Envelope;

impl Service {
    /// Executes a request on the calling thread (callers use `spawn_blocking`).
    pub fn execute(&self, req: Request) -> Envelope {
        self.execute_with_actor(req, None)
    }

    /// `preresolved` lets the job runner replay a workflow step as its original requester.
    pub fn execute_with_actor(&self, req: Request, preresolved: Option<Actor>) -> Envelope {
        let request_id = req.request_id.clone().unwrap_or_else(openclaim_db::new_id);
        let trace_id = req.trace_id.clone().unwrap_or_else(openclaim_db::new_id);
        let started = Instant::now();
        let now = openclaim_domain::dates::now_rfc3339();
        let mut guard = match self.inner.db.lock() {
            Ok(g) => g,
            Err(_) => return Envelope::failure(request_id, trace_id, OcError::internal("database lock poisoned")),
        };
        let Some(op) = ops::find(&req.op) else {
            return Envelope::failure(request_id, trace_id, OcError::invalid(format!("unknown operation '{}'", req.op)).with_next_action("run `openclaim schema commands`"));
        };
        let tx = match guard.conn.transaction() {
            Ok(t) => t,
            Err(e) => return Envelope::failure(request_id, trace_id, openclaim_db::db_err(e)),
        };
        let actor = match preresolved {
            Some(a) => match authz::recheck_actor(&tx, a, &now) {
                Ok(a) => a,
                Err(e) => return Envelope::failure(request_id, trace_id, e),
            },
            None => match authz::resolve_actor(self, &tx, &req.credentials, &now, &req.transport) {
                Ok(a) => a,
                Err(e) => return Envelope::failure(request_id, trace_id, e),
            },
        };
        if op.name == "data.restore" {
            drop(tx);
            return self.special_restore(&mut guard, actor, req, request_id, trace_id, started);
        }
        let actor_id = actor.id();

        // Idempotent replay: same key + same request → same answer, without re-running.
        let request_hash = json_digest(&json!({"op": req.op, "params": req.params, "dry_run": req.dry_run}));
        if let Some(key) = req.idempotency_key.as_deref().filter(|k| !k.is_empty()) {
            if op.effect.is_mutation() && !req.dry_run {
                match query_one(&tx, "SELECT request_hash, response_json FROM idempotency_keys WHERE actor_id = ?1 AND command_scope = ?2 AND key = ?3 AND expires_at > ?4", (&actor_id, op.name, key, &now)) {
                    Ok(Some(row)) => {
                        if row["request_hash"].as_str() == Some(&request_hash) {
                            if let Ok(env) = serde_json::from_value::<Envelope>(row["response"].clone()) {
                                let mut env = env;
                                env.warnings.push("replayed from idempotency key".into());
                                return env;
                            }
                        } else {
                            return Envelope::failure(request_id, trace_id, OcError::conflict("idempotency key was used with different parameters"));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => return Envelope::failure(request_id, trace_id, e),
                }
            }
        }

        let authorized = match authz::authorize(&tx, &actor, op, &req.params, &req, &now) {
            Ok(a) => a,
            Err(e) => {
                // Approval requests are persisted even though the command did not run.
                if e.code == ErrorCode::ApprovalRequired {
                    let _ = record_run(&tx, &actor_id, op.name, &req.params, &request_id, "approval_required", req.dry_run, started);
                    let _ = tx.commit();
                    drop(guard);
                    self.broadcast_pending();
                } else {
                    let _ = record_run(&tx, &actor_id, op.name, &req.params, &request_id, "denied", req.dry_run, started);
                    let _ = tx.commit();
                }
                return Envelope::failure(request_id, trace_id, e);
            }
        };

        let mut ctx = Ctx {
            conn: &tx,
            svc: self,
            actor: actor.clone(),
            dry_run: req.dry_run,
            offline: req.offline,
            non_interactive: req.non_interactive,
            now: now.clone(),
            request_id: request_id.clone(),
            trace_id: trace_id.clone(),
            transport: req.transport.clone(),
            warnings: Vec::new(),
            jobs_enqueued: false,
        };
        let result = (op.handler)(&mut ctx, req.params.clone());
        let warnings = std::mem::take(&mut ctx.warnings);
        let jobs_enqueued = ctx.jobs_enqueued;
        drop(ctx);

        match result {
            Ok(mut data) => {
                let pagination = lift_pagination(&mut data);
                let mut env = Envelope::success(request_id.clone(), trace_id.clone(), data);
                env.warnings = warnings;
                env.pagination = pagination;
                env.dry_run = req.dry_run;
                if req.dry_run {
                    env.warnings.push("dry run: no changes were committed".into());
                    if op.network {
                        env.warnings.push("dry run: the network fetch this command would start was not started".into());
                    }
                    let _ = tx.rollback();
                    let _ = record_run_fresh(&mut guard.conn, &actor_id, op.name, &req.params, &request_id, "dry_run", true, started);
                    return env;
                }
                if op.effect.is_mutation() {
                    match crate::audit::append(&tx, &actor_id, op.name, &subject_of(&req.params), json!({"request_id": request_id, "approval": authorized.consumed_approval})) {
                        Ok(id) => env.audit_event_id = Some(id),
                        Err(e) => return Envelope::failure(request_id, trace_id, e),
                    }
                }
                let _ = record_run(&tx, &actor_id, op.name, &req.params, &request_id, "ok", false, started);
                if let Some(key) = req.idempotency_key.as_deref().filter(|k| !k.is_empty()) {
                    if op.effect.is_mutation() {
                        let expires = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::hours(24));
                        let _ = exec(
                            &tx,
                            "INSERT OR REPLACE INTO idempotency_keys(actor_id, command_scope, key, request_hash, response_json, expires_at) VALUES (?1,?2,?3,?4,?5,?6)",
                            (&actor_id, op.name, key, &request_hash, serde_json::to_string(&env).unwrap_or_default(), expires),
                        );
                    }
                }
                if let Err(e) = tx.commit() {
                    return Envelope::failure(request_id, trace_id, openclaim_db::db_err(e));
                }
                drop(guard);
                self.broadcast_pending();
                if jobs_enqueued {
                    self.inner.job_wake.notify_one();
                }
                env
            }
            Err(e) => {
                let _ = tx.rollback();
                let _ = record_run_fresh(&mut guard.conn, &actor_id, op.name, &req.params, &request_id, &format!("error:{}", e.code.as_str()), req.dry_run, started);
                let mut env = Envelope::failure(request_id, trace_id, e);
                env.warnings = warnings;
                env
            }
        }
    }

    /// Runs an operation as the daemon itself (scheduler, job runner).
    pub fn execute_system(&self, op: &str, params: Value) -> Envelope {
        let mut req = Request::new(op, params);
        req.transport = "system".into();
        self.execute_as(req, Actor::System)
    }

    /// Runs with an already-resolved actor (workflow steps replaying their requester).
    pub fn execute_as(&self, mut req: Request, actor: Actor) -> Envelope {
        match &actor {
            Actor::System => {
                req.credentials.session = Some(self.system_session());
                self.execute(req)
            }
            Actor::Owner { session_id } => {
                req.credentials.session = Some(session_id.clone());
                self.execute(req)
            }
            Actor::Agent { .. } | Actor::Anonymous => self.execute(req),
        }
    }

    fn system_session(&self) -> String {
        let mut sessions = self.inner.sessions.lock().expect("session lock");
        let id = format!("system-{}", self.inner.instance_id);
        sessions.entry(id.clone()).or_insert(crate::OwnerSession { id: id.clone(), expires_at: time::OffsetDateTime::now_utc() + time::Duration::days(3650), transport: "system".into() });
        id
    }

    /// Pushes committed outbox events to live subscribers.
    pub fn broadcast_pending(&self) {
        let Ok(guard) = self.inner.db.lock() else { return };
        let last: i64 = crate::sql::scalar(&guard.conn, "SELECT value FROM meta WHERE key = 'events_broadcast_seq'", []).ok().flatten().and_then(|s: String| s.parse().ok()).unwrap_or(0);
        let Ok(events) = crate::events::since(&guard.conn, last, 500) else { return };
        let mut max = last;
        for e in events {
            max = max.max(e.seq);
            let _ = self.inner.events.send(e);
        }
        if max != last {
            let _ = exec(&guard.conn, "INSERT INTO meta(key, value) VALUES('events_broadcast_seq', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [max.to_string()]);
        }
    }
}

impl Service {
    fn special_restore(&self, guard: &mut std::sync::MutexGuard<'_, openclaim_db::Database>, actor: Actor, req: Request, request_id: String, trace_id: String, started: Instant) -> Envelope {
        if !actor.is_owner() {
            return Envelope::failure(request_id, trace_id, OcError::denied("data.restore is owner-only"));
        }
        let path = req.params["path"].as_str().unwrap_or("").to_string();
        if req.params["confirm"] != true {
            return Envelope::failure(request_id, trace_id, OcError::invalid("pass confirm=true to replace the live database"));
        }
        if req.dry_run {
            let mut env = Envelope::success(request_id, trace_id, json!({"would_restore_from": path}));
            env.dry_run = true;
            return env;
        }
        let src = match openclaim_db::rusqlite::Connection::open_with_flags(&path, openclaim_db::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(e) => return Envelope::failure(request_id, trace_id, OcError::invalid(format!("cannot open backup: {e}"))),
        };
        let version: Result<String, _> = src.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0));
        let Ok(version) = version else {
            return Envelope::failure(request_id, trace_id, OcError::invalid("not an OpenClaim database"));
        };
        {
            let backup = match openclaim_db::rusqlite::backup::Backup::new(&src, &mut guard.conn) {
                Ok(b) => b,
                Err(e) => return Envelope::failure(request_id, trace_id, openclaim_db::db_err(e)),
            };
            if let Err(e) = backup.run_to_completion(256, std::time::Duration::from_millis(5), None) {
                return Envelope::failure(request_id, trace_id, openclaim_db::db_err(e));
            }
        }
        let vault_src = std::path::Path::new(&path).with_extension("vault");
        let mut files = 0;
        if vault_src.is_dir() {
            for e in std::fs::read_dir(&vault_src).into_iter().flatten().flatten() {
                if e.path().is_file() && std::fs::copy(e.path(), self.inner.paths.vault_dir.join(e.file_name())).is_ok() {
                    files += 1;
                }
            }
        }
        let _ = record_run_fresh(&mut guard.conn, &actor.id(), "data.restore", &req.params, &request_id, "ok", false, started);
        let _ = guard.conn.execute("INSERT INTO audit_log(id, actor, action, timestamp, redacted_subject, details_json, prev_hash, integrity_hash) SELECT ?1, ?2, 'data.restore', ?3, ?4, '{}', COALESCE((SELECT integrity_hash FROM audit_log ORDER BY seq DESC LIMIT 1), 'genesis'), ?5", (openclaim_db::new_id(), actor.id(), openclaim_domain::dates::now_rfc3339(), &path, openclaim_security::digest::sha256_hex(path.as_bytes())));
        Envelope::success(request_id, trace_id, json!({"restored_from": path, "schema_version": version, "vault_files": files, "note": "restart clients; in-memory sessions were kept"}))
    }
}

fn subject_of(params: &Value) -> String {
    for k in ["claim_id", "person_id", "opportunity_id", "document_id", "source_id", "job_id", "approval_id", "agent_id", "id"] {
        if let Some(v) = params.get(k).and_then(|v| v.as_str()) {
            return format!("{k}={v}");
        }
    }
    "-".into()
}

fn lift_pagination(data: &mut Value) -> Option<Pagination> {
    let obj = data.as_object_mut()?;
    let p = obj.remove("pagination")?;
    serde_json::from_value(p).ok()
}

#[allow(clippy::too_many_arguments)]
fn record_run(conn: &openclaim_db::rusqlite::Connection, actor: &str, op: &str, params: &Value, request_id: &str, result: &str, dry_run: bool, started: Instant) -> OcResult<()> {
    exec(
        conn,
        "INSERT INTO command_runs(id, actor_id, command, args_redacted_json, request_id, result, dry_run, duration_ms, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        (
            openclaim_db::new_id(),
            actor,
            op,
            redact_json(params).to_string(),
            request_id,
            result,
            dry_run as i32,
            started.elapsed().as_millis() as i64,
            openclaim_domain::dates::now_rfc3339(),
        ),
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_run_fresh(conn: &mut openclaim_db::rusqlite::Connection, actor: &str, op: &str, params: &Value, request_id: &str, result: &str, dry_run: bool, started: Instant) -> OcResult<()> {
    let tx = conn.transaction().map_err(openclaim_db::db_err)?;
    record_run(&tx, actor, op, params, request_id, result, dry_run, started)?;
    tx.commit().map_err(openclaim_db::db_err)
}

pub fn effect_label(e: Effect) -> &'static str {
    match e {
        Effect::Read => "read",
        Effect::Write => "write",
        Effect::Elevated => "elevated",
        Effect::OwnerOnly => "owner_only",
    }
}
