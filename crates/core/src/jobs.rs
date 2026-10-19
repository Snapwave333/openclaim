//! Durable job queue and scheduler (MDR §13, §21.2): SQLite-backed state,
//! leases, retries with backoff, cancellation points, crash recovery.

use crate::ctx::{Actor, Ctx};
use crate::sql::{exec, query_all, query_one, scalar};
use crate::{Service, modules};
use openclaim_db::rusqlite::Connection;
use openclaim_domain::model::JobState;
use openclaim_domain::{ErrorCode, OcError, OcResult};
use serde_json::{Value, json};
use std::time::Duration;

pub const KINDS: &[(&str, &str)] = &[
    ("source_refresh", "Fetch and ingest one permitted source"),
    ("source_test", "One-request connectivity test for a source"),
    ("match_refresh", "Re-evaluate eligibility for stale matches"),
    ("document_extract", "On-device text extraction for a vault document"),
    ("alert_sweep", "Deliver due reminders, expire approvals and sessions"),
    ("stale_sweep", "Flag verified records whose source went unreachable"),
    ("integrity_check", "Database integrity and audit-chain verification"),
    ("workflow_run", "Execute a declarative workflow run"),
];

const LEASE_SECS: i64 = 600;

pub fn enqueue(ctx: &mut Ctx<'_>, kind: &str, params: Value, parent: Option<&str>) -> OcResult<String> {
    let id = enqueue_conn(ctx.conn, kind, params, &ctx.actor_id(), parent, &ctx.now)?;
    ctx.jobs_enqueued = true;
    Ok(id)
}

pub fn enqueue_conn(conn: &Connection, kind: &str, params: Value, actor_id: &str, parent: Option<&str>, now: &str) -> OcResult<String> {
    if !KINDS.iter().any(|(k, _)| *k == kind) {
        return Err(OcError::invalid(format!("unknown job kind '{kind}'")));
    }
    // Collapse duplicates: an identical queued job is reused.
    let digest = openclaim_security::digest::json_digest(&json!({"kind": kind, "params": params}));
    if let Some(existing) = scalar::<String, _>(conn, "SELECT id FROM jobs WHERE state = 'queued' AND cursor = ?1", [&digest])? {
        return Ok(existing);
    }
    let id = openclaim_db::new_id();
    let max_attempts = if kind == "source_refresh" { 3 } else { 2 };
    exec(
        conn,
        "INSERT INTO jobs(id, kind, state, params_json, owner_actor_id, parent_job_id, next_run_at, max_attempts, cursor, created_at, updated_at) VALUES (?1,?2,'queued',?3,?4,?5,?6,?7,?8,?6,?6)",
        (&id, kind, params.to_string(), actor_id, parent, now, max_attempts, &digest),
    )?;
    job_event(conn, &id, "job.queued", json!({"kind": kind}), now)?;
    Ok(id)
}

pub fn job_event(conn: &Connection, job_id: &str, event_type: &str, payload: Value, now: &str) -> OcResult<()> {
    let seq: i64 = scalar::<i64, _>(conn, "SELECT COALESCE(max(sequence),0) FROM job_events WHERE job_id = ?1", [job_id])?.unwrap_or(0) + 1;
    let payload = openclaim_security::redact::redact_json(&payload);
    exec(
        conn,
        "INSERT INTO job_events(id, job_id, sequence, event_type, payload_redacted_json, at) VALUES (?1,?2,?3,?4,?5,?6)",
        (openclaim_db::new_id(), job_id, seq, event_type, payload.to_string(), now),
    )?;
    let mut ev = json!({"event": event_type, "job_id": job_id, "sequence": seq, "at": now});
    if let (Some(e), Some(p)) = (ev.as_object_mut(), payload.as_object()) {
        for (k, v) in p {
            e.insert(k.clone(), v.clone());
        }
    }
    crate::events::emit(conn, event_type, ev)?;
    Ok(())
}

pub fn seed_schedules(conn: &Connection) -> OcResult<()> {
    let now = openclaim_domain::dates::now_rfc3339();
    let defaults: &[(&str, &str, i64)] = &[
        ("sched-sources", "source_refresh_all", 3600),
        ("sched-alerts", "alert_sweep", 900),
        ("sched-stale", "stale_sweep", 6 * 3600),
        ("sched-integrity", "integrity_check", 7 * 24 * 3600),
    ];
    for (id, kind, secs) in defaults {
        exec(
            conn,
            "INSERT OR IGNORE INTO schedules(id, kind, params_json, interval_secs, next_run_at, enabled) VALUES (?1,?2,'{}',?3,?4,1)",
            (id, kind, secs, &now),
        )?;
    }
    Ok(())
}

fn cancelled(conn: &Connection, job_id: &str) -> bool {
    scalar::<i64, _>(conn, "SELECT cancel_requested FROM jobs WHERE id = ?1", [job_id])
        .ok()
        .flatten()
        .unwrap_or(0)
        != 0
}

impl Service {
    fn with_tx<T>(&self, f: impl FnOnce(&Connection, &str) -> OcResult<T>) -> OcResult<T> {
        let mut guard = self.inner.db.lock().map_err(|_| OcError::internal("db lock poisoned"))?;
        let tx = guard.conn.transaction().map_err(openclaim_db::db_err)?;
        let now = openclaim_domain::dates::now_rfc3339();
        let out = f(&tx, &now)?;
        tx.commit().map_err(openclaim_db::db_err)?;
        drop(guard);
        self.broadcast_pending();
        Ok(out)
    }

    fn with_ctx<T>(&self, f: impl FnOnce(&mut Ctx<'_>) -> OcResult<T>) -> OcResult<T> {
        self.with_tx(|conn, now| {
            let mut ctx = Ctx {
                conn,
                svc: self,
                actor: Actor::System,
                dry_run: false,
                offline: false,
                non_interactive: true,
                now: now.to_string(),
                request_id: openclaim_db::new_id(),
                trace_id: openclaim_db::new_id(),
                transport: "system".into(),
                warnings: Vec::new(),
                jobs_enqueued: false,
            };
            let out = f(&mut ctx)?;
            if ctx.jobs_enqueued {
                self.inner.job_wake.notify_one();
            }
            Ok(out)
        })
    }

    /// Requeues jobs whose lease expired (crash recovery).
    pub fn recover_jobs(&self) -> OcResult<u32> {
        self.with_tx(|conn, now| {
            let n = exec(conn, "UPDATE jobs SET state = 'queued', lease_owner = NULL, lease_expires_at = NULL, updated_at = ?1 WHERE state = 'running' AND (lease_expires_at IS NULL OR lease_expires_at < ?1)", [now])?;
            Ok(n as u32)
        })
    }

    fn tick_schedules(&self) -> OcResult<()> {
        self.with_tx(|conn, now| {
            for s in query_all(
                conn,
                "SELECT id, kind, params_json, interval_secs FROM schedules WHERE enabled = 1 AND next_run_at <= ?1",
                [now],
            )? {
                let kind = s["kind"].as_str().unwrap_or("");
                let next =
                    openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::seconds(s["interval_secs"].as_i64().unwrap_or(3600)));
                exec(
                    conn,
                    "UPDATE schedules SET next_run_at = ?2 WHERE id = ?1",
                    (s["id"].as_str().unwrap_or(""), next),
                )?;
                if kind == "source_refresh_all" {
                    if modules::config::get_bool(conn, "network.offline") {
                        continue;
                    }
                    for src in query_all(
                        conn,
                        "SELECT id FROM sources WHERE enabled = 1 AND health != 'needs_configuration' AND (next_fetch_at IS NULL OR next_fetch_at <= ?1)",
                        [now],
                    )? {
                        enqueue_conn(conn, "source_refresh", json!({"source_id": src["id"], "force": false}), "system", None, now)?;
                    }
                } else {
                    enqueue_conn(conn, kind, s["params"].clone(), "system", None, now)?;
                }
            }
            Ok(())
        })
    }

    fn claim_job(&self, worker: &str) -> OcResult<Option<Value>> {
        self.with_tx(|conn, now| {
            let lease = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::seconds(LEASE_SECS));
            let row = query_one(
                conn,
                "UPDATE jobs SET state = 'running', lease_owner = ?1, lease_expires_at = ?2, attempts = attempts + 1, updated_at = ?3
                 WHERE id = (SELECT id FROM jobs WHERE state = 'queued' AND next_run_at <= ?3 AND cancel_requested = 0 ORDER BY created_at LIMIT 1)
                 RETURNING id, kind, params_json, attempts, max_attempts, owner_actor_id",
                (worker, lease, now),
            )?;
            if let Some(r) = &row {
                job_event(conn, r["id"].as_str().unwrap_or(""), "job.started", json!({"attempt": r["attempts"]}), now)?;
            }
            Ok(row)
        })
    }

    pub fn job_progress(&self, job_id: &str, percent: u32, note: &str) -> OcResult<()> {
        self.with_tx(|conn, now| {
            let lease = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::seconds(LEASE_SECS));
            exec(
                conn,
                "UPDATE jobs SET progress = ?2, lease_expires_at = ?3, updated_at = ?4 WHERE id = ?1",
                (job_id, percent as i64, lease, now),
            )?;
            job_event(conn, job_id, "job.progress", json!({"percent": percent, "note": note}), now)
        })
    }

    fn finish_job(&self, job_id: &str, result: Result<Value, OcError>, attempts: i64, max_attempts: i64) -> OcResult<()> {
        self.with_tx(|conn, now| {
            match result {
                Ok(v) => {
                    exec(conn, "UPDATE jobs SET state = 'succeeded', progress = 100, result_json = ?2, finished_at = ?3, updated_at = ?3, lease_owner = NULL WHERE id = ?1", (job_id, v.to_string(), now))?;
                    job_event(conn, job_id, "job.completed", json!({"result_ref": job_id}), now)
                }
                Err(e) if e.code == ErrorCode::ApprovalRequired => {
                    exec(conn, "UPDATE jobs SET state = 'paused_for_approval', error_code = 'APPROVAL_REQUIRED', error_message = ?2, updated_at = ?3, lease_owner = NULL WHERE id = ?1", (job_id, &e.message, now))?;
                    job_event(conn, job_id, "job.paused", json!({"approval_id": e.approval_id}), now)
                }
                Err(e) => {
                    let retry = e.retryable && attempts < max_attempts;
                    if retry {
                        let delay = 30i64 * 2i64.pow(attempts.clamp(1, 8) as u32);
                        let next = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::seconds(delay));
                        exec(conn, "UPDATE jobs SET state = 'queued', next_run_at = ?2, error_code = ?3, error_message = ?4, updated_at = ?5, lease_owner = NULL WHERE id = ?1", (job_id, next, e.code.as_str(), &e.message, now))?;
                        job_event(conn, job_id, "job.retry_scheduled", json!({"delay_seconds": delay, "error": e.message}), now)
                    } else {
                        exec(conn, "UPDATE jobs SET state = 'failed', error_code = ?2, error_message = ?3, finished_at = ?4, updated_at = ?4, lease_owner = NULL WHERE id = ?1", (job_id, e.code.as_str(), &e.message, now))?;
                        job_event(conn, job_id, "job.failed", json!({"error_code": e.code.as_str(), "error": e.message}), now)
                    }
                }
            }
        })
    }

    /// Main worker loop. Returns when the daemon is asked to stop.
    pub async fn run_jobs(&self) {
        let _ = self.recover_jobs();
        let mut running: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        loop {
            if self.inner.stop_requested.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            running.retain(|h| !h.is_finished());
            let _ = self.tick_schedules();
            let concurrency = self
                .inner
                .db
                .lock()
                .ok()
                .map(|g| modules::config::get_u64(&g.conn, "jobs.concurrency", 2))
                .unwrap_or(2) as usize;
            while running.len() < concurrency {
                let worker = format!("{}-{}", self.inner.instance_id, running.len());
                match self.claim_job(&worker) {
                    Ok(Some(job)) => {
                        let svc = self.clone();
                        running.push(tokio::spawn(async move { svc.run_one(job).await }));
                    }
                    _ => break,
                }
            }
            tokio::select! {
                _ = self.inner.job_wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                _ = self.inner.shutdown.notified() => break,
            }
        }
        for h in running {
            let _ = tokio::time::timeout(Duration::from_secs(30), h).await;
        }
    }

    /// Runs every runnable job to completion on the current task (tests and `jobs wait`).
    pub async fn drain_jobs(&self) -> u32 {
        let mut n = 0;
        while let Ok(Some(job)) = self.claim_job("drain") {
            self.run_one(job).await;
            n += 1;
            if n > 200 {
                break;
            }
        }
        n
    }

    async fn run_one(&self, job: Value) {
        let id = job["id"].as_str().unwrap_or("").to_string();
        let kind = job["kind"].as_str().unwrap_or("").to_string();
        let params = job["params"].clone();
        let attempts = job["attempts"].as_i64().unwrap_or(1);
        let max_attempts = job["max_attempts"].as_i64().unwrap_or(1);
        let result = match kind.as_str() {
            "source_refresh" => self.job_source_refresh(&id, &params).await,
            "source_test" => self.job_source_test(&params).await,
            "match_refresh" => self.job_match_refresh(&params),
            "document_extract" => self.job_document_extract(&params),
            "alert_sweep" => self.job_alert_sweep(),
            "stale_sweep" => self.with_tx(|c, now| Ok(json!({"marked_stale": modules::sources::mark_stale(c, now)?}))),
            "integrity_check" => self.job_integrity(),
            "workflow_run" => self.job_workflow_run(&id, &params).await,
            other => Err(OcError::internal(format!("no handler for job kind {other}"))),
        };
        let _ = self.finish_job(&id, result, attempts, max_attempts);
    }

    async fn job_source_refresh(&self, job_id: &str, params: &Value) -> OcResult<Value> {
        let source_id = params["source_id"].as_str().unwrap_or("").to_string();
        let (manifest, etag, last_modified, contact, offline, due) = {
            let guard = self.inner.db.lock().map_err(|_| OcError::internal("db lock"))?;
            let row = crate::sql::require(
                query_one(
                    &guard.conn,
                    "SELECT enabled, etag, last_modified, manifest_json FROM sources WHERE id = ?1",
                    [&source_id],
                )?,
                "source",
                &source_id,
            )?;
            if row["enabled"] != true {
                return Err(OcError::policy("source is disabled"));
            }
            let m: openclaim_discovery::ConnectorManifest = serde_json::from_value(row["manifest"].clone())?;
            let due = modules::sources::detail_due(&guard.conn, &source_id, m.max_detail_pages as usize)?;
            (
                m,
                row["etag"].as_str().map(str::to_string),
                row["last_modified"].as_str().map(str::to_string),
                modules::config::get_str(&guard.conn, "network.contact"),
                modules::config::get_bool(&guard.conn, "network.offline"),
                due,
            )
        };
        let force = params["force"].as_bool().unwrap_or(false);
        let input = openclaim_discovery::run::RunInput {
            manifest: manifest.clone(),
            prior_etag: if force { None } else { etag },
            prior_last_modified: if force { None } else { last_modified },
            offline,
            contact,
            today: openclaim_domain::dates::today_utc(),
            detail_due: due,
        };
        self.job_progress(job_id, 10, "fetching index")?;
        let out = openclaim_discovery::run::run(input).await;
        self.job_progress(job_id, 70, "persisting")?;
        let sid = source_id.clone();
        let outcome: Result<Result<Value, OcError>, OcError> = self.with_tx(move |conn, now| {
            if cancelled(conn, job_id) {
                return Err(OcError::new(ErrorCode::Timeout, "cancelled before persisting"));
            }
            let mut snapshot_ids = Vec::new();
            for d in &out.docs {
                let (sid_snap, _) = modules::sources::record_snapshot(conn, &sid, &d.url, &d.body, "fetch", Some(d.status), now)?;
                snapshot_ids.push(sid_snap);
            }
            let snapshot_id = snapshot_ids.first().cloned().unwrap_or_else(|| "none".into());
            let summary = if out.opportunities.is_empty() && out.programs.is_empty() && out.tips.is_empty() {
                modules::sources::IngestSummary::default()
            } else {
                modules::sources::ingest(conn, &sid, &snapshot_id, manifest.authority_tier, &out.opportunities, &out.programs, &out.tips, now)?
            };
            match &out.failure {
                Some((class, detail, retry_after)) => {
                    modules::sources::record_failure(conn, &sid, *class, detail, *retry_after, now)?;
                    let mut e = OcError::new(if matches!(class, openclaim_domain::model::FetchFailure::AntiBotBlock | openclaim_domain::model::FetchFailure::RobotsDisallowed | openclaim_domain::model::FetchFailure::PolicyBlocked | openclaim_domain::model::FetchFailure::Offline) { ErrorCode::PolicyBlocked } else { ErrorCode::Unavailable }, format!("{}: {detail}", class.as_str()));
                    e.retryable = matches!(class, openclaim_domain::model::FetchFailure::NetworkError | openclaim_domain::model::FetchFailure::HttpError);
                    e.source_status = Some(class.as_str().to_string());
                    e.next_action = Some(match class {
                        openclaim_domain::model::FetchFailure::AntiBotBlock => format!("open {} in your browser, save the page, then `openclaim sources import-snapshot --source {sid} --url <page-url> --file <saved.html>`", manifest.url),
                        openclaim_domain::model::FetchFailure::PolicyBlocked => "configure network.contact or check the manifest".into(),
                        _ => "retry later; the official site stays the authority".into(),
                    });
                    e.details = Some(json!({"ingested": summary, "notes": out.notes, "requests": out.requests}));
                    Ok(Err(e))
                }
                None => {
                    modules::sources::record_success(conn, &sid, out.etag.clone(), out.last_modified.clone(), manifest.freshness_ttl_hours, now)?;
                    if summary.inserted + summary.changes > 0 {
                        enqueue_conn(conn, "match_refresh", json!({}), "system", Some(job_id), now)?;
                    }
                    Ok(Ok(json!({"source_id": sid, "not_modified": out.not_modified, "requests": out.requests, "ingested": summary, "detail_pages": out.detail_fetched.len(), "notes": out.notes})))
                }
            }
        });
        outcome?
    }

    async fn job_source_test(&self, params: &Value) -> OcResult<Value> {
        let source_id = params["source_id"].as_str().unwrap_or("").to_string();
        let (manifest, contact, offline) = {
            let guard = self.inner.db.lock().map_err(|_| OcError::internal("db lock"))?;
            (
                modules::sources::manifest_of(&guard.conn, &source_id)?,
                modules::config::get_str(&guard.conn, "network.contact"),
                modules::config::get_bool(&guard.conn, "network.offline"),
            )
        };
        let fetcher = openclaim_discovery::fetch::Fetcher::new(openclaim_discovery::run::fetch_policy(&manifest, offline, contact.as_deref()))
            .map_err(OcError::internal)?;
        match fetcher.get(&manifest.url, None).await {
            Ok(f) => Ok(json!({"source_id": source_id, "ok": true, "http_status": f.status, "bytes": f.body.len(), "final_url": f.url})),
            Err(e) => {
                Ok(json!({"source_id": source_id, "ok": false, "failure_class": e.class.as_str(), "detail": e.message, "retry_after": e.retry_after_secs}))
            }
        }
    }

    fn job_match_refresh(&self, params: &Value) -> OcResult<Value> {
        self.with_tx(|conn, now| {
            let people: Vec<String> = match params["person_id"].as_str() {
                Some(p) => vec![p.to_string()],
                None => query_all(conn, "SELECT id FROM people", [])?
                    .into_iter()
                    .filter_map(|r| r["id"].as_str().map(str::to_string))
                    .collect(),
            };
            let mut evaluated = 0;
            for pid in &people {
                if let Some(oid) = params["opportunity_id"].as_str() {
                    modules::matching::evaluate_pair(conn, pid, oid, now)?;
                    evaluated += 1;
                } else {
                    evaluated += modules::matching::evaluate_person(conn, pid, true, now)?.len();
                }
            }
            Ok(json!({"people": people.len(), "evaluated": evaluated}))
        })
    }

    fn job_document_extract(&self, params: &Value) -> OcResult<Value> {
        let id = params["document_id"].as_str().unwrap_or("").to_string();
        let (extraction, index) = modules::documents::extract_bytes(self, &id)?;
        self.with_ctx(|ctx| {
            modules::documents::store_extraction(ctx, &id, &extraction, index)?;
            Ok(json!({"document_id": id, "state": extraction.state, "classification": extraction.classification, "suggestions": extraction.suggestions.len(), "chars": extraction.text.chars().count()}))
        })
    }

    fn job_alert_sweep(&self) -> OcResult<Value> {
        let expired_sessions = {
            let mut s = self.inner.sessions.lock().map_err(|_| OcError::internal("lock"))?;
            let before = s.len();
            let now = time::OffsetDateTime::now_utc();
            s.retain(|_, v| v.expires_at > now);
            before - s.len()
        };
        self.with_tx(|conn, now| {
            let delivered = modules::alerts::sweep(conn, now)?;
            let expired = exec(
                conn,
                "UPDATE agent_approvals SET decision = 'expired' WHERE decision = 'pending' AND expires_at <= ?1",
                [now],
            )?;
            exec(conn, "DELETE FROM idempotency_keys WHERE expires_at <= ?1", [now])?;
            exec(
                conn,
                "DELETE FROM events WHERE at < ?1",
                [openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() - time::Duration::days(30))],
            )?;
            Ok(json!({"alerts_delivered": delivered, "approvals_expired": expired, "sessions_expired": expired_sessions}))
        })
    }

    fn job_integrity(&self) -> OcResult<Value> {
        let problems = {
            let guard = self.inner.db.lock().map_err(|_| OcError::internal("db lock"))?;
            guard.integrity()?
        };
        self.with_tx(|conn, now| {
            let (count, broken) = crate::audit::verify(conn)?;
            exec(conn, "INSERT INTO meta(key, value) VALUES('last_integrity_check', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [now])?;
            if !problems.is_empty() || broken.is_some() {
                crate::events::emit(conn, "integrity.problem", json!({"problems": problems, "audit_broken_at": broken}))?;
            }
            Ok(json!({"database_problems": problems, "audit_entries": count, "audit_broken_at": broken, "backup_hint": "run `openclaim data backup <path>` regularly"}))
        })
    }

    async fn job_workflow_run(&self, job_id: &str, params: &Value) -> OcResult<Value> {
        let run_id = params["run_id"].as_str().unwrap_or("").to_string();
        modules::workflows::execute_run(self, job_id, &run_id).await
    }
}

pub fn state_of(s: &str) -> Option<JobState> {
    JobState::parse(s)
}
