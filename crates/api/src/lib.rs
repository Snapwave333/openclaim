//! Localhost REST API, SSE events and static GUI hosting (MDR §6, §11, §12).
//!
//! Bound to 127.0.0.1 only. Defenses: Host allowlist (DNS rebinding), Origin
//! allowlist, SameSite=Strict HttpOnly session cookie plus a CSRF header for
//! cookie-authenticated mutations, strict CSP, bounded bodies.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, Request as HttpRequest, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response, sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use openclaim_core::{Credentials, Request, Service};
use openclaim_domain::{Envelope, OcError};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const COOKIE: &str = "oc_session";
pub const CSRF_HEADER: &str = "x-openclaim-csrf";

#[derive(Clone)]
pub struct AppState {
    pub svc: Service,
    pub port: u16,
    pub gui_dir: Option<PathBuf>,
    /// session token → csrf token
    pub csrf: Arc<Mutex<HashMap<String, String>>>,
}

pub struct ApiError(Envelope);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.0.error.as_ref().map(|e| e.code.http_status()).unwrap_or(500);
        (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(self.0)).into_response()
    }
}

fn fail(e: OcError) -> ApiError {
    ApiError(Envelope::failure(openclaim_core::new_request_id(), openclaim_core::new_request_id(), e))
}

fn allowed_hosts(port: u16) -> Vec<String> {
    vec![format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
}

fn extension_origins(svc: &Service) -> Vec<String> {
    let Ok(guard) = svc.inner.db.lock() else { return vec![] };
    openclaim_core::sql::query_all(
        &guard.conn,
        "SELECT allowed_origin FROM agent_principals WHERE kind = 'extension' AND revoked_at IS NULL AND allowed_origin IS NOT NULL",
        [],
    )
    .map(|rows| rows.into_iter().filter_map(|r| r["allowed_origin"].as_str().map(str::to_string)).collect())
    .unwrap_or_default()
}

/// Host + Origin checks, security headers, CORS for paired extensions.
async fn guard(State(st): State<AppState>, req: HttpRequest<Body>, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("").to_string();
    if !allowed_hosts(st.port).iter().any(|h| h == &host) {
        return (StatusCode::FORBIDDEN, "host not allowed").into_response();
    }
    let origin = req.headers().get(header::ORIGIN).and_then(|h| h.to_str().ok()).map(str::to_string);
    let mut local_origins: Vec<String> = allowed_hosts(st.port).iter().map(|h| format!("http://{h}")).collect();
    // Development only: `OPENCLAIM_DEV_ORIGINS=http://localhost:4200` lets `ng serve` proxy to the daemon.
    if let Ok(extra) = std::env::var("OPENCLAIM_DEV_ORIGINS") {
        local_origins.extend(
            extra
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| s.starts_with("http://localhost:") || s.starts_with("http://127.0.0.1:")),
        );
    }
    let mut cors_origin: Option<String> = None;
    if let Some(o) = &origin {
        if local_origins.contains(o) {
            // same-site GUI
        } else if extension_origins(&st.svc).contains(o) {
            cors_origin = Some(o.clone());
        } else {
            return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
        }
    }
    if req.method() == Method::OPTIONS {
        let mut resp = StatusCode::NO_CONTENT.into_response();
        if let Some(o) = cors_origin {
            apply_cors(resp.headers_mut(), &o);
        }
        return resp;
    }
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    h.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"),
    );
    if let Some(o) = cors_origin {
        apply_cors(h, &o);
    }
    resp
}

fn apply_cors(h: &mut HeaderMap, origin: &str) {
    if let Ok(v) = HeaderValue::from_str(origin) {
        h.insert("access-control-allow-origin", v);
    }
    h.insert("access-control-allow-methods", HeaderValue::from_static("GET, POST, PATCH, DELETE, OPTIONS"));
    h.insert("access-control-allow-headers", HeaderValue::from_static("authorization, content-type"));
    h.insert("vary", HeaderValue::from_static("Origin"));
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix(&format!("{COOKIE}=")).map(str::to_string))
}

/// Builds credentials from a Bearer agent token or the session cookie (with CSRF for mutations).
fn credentials(st: &AppState, headers: &HeaderMap, mutation: bool) -> Result<Credentials, ApiError> {
    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok())
        && let Some(tok) = auth.strip_prefix("Bearer ")
    {
        return Ok(Credentials {
            session: None,
            agent_token: Some(tok.trim().to_string()),
        });
    }
    if let Some(session) = cookie_value(headers) {
        if mutation {
            let expected = st.csrf.lock().ok().and_then(|m| m.get(&session).cloned());
            let presented = headers.get(CSRF_HEADER).and_then(|h| h.to_str().ok()).map(str::to_string);
            match (expected, presented) {
                (Some(e), Some(p)) if openclaim_security::tokens::ct_eq(&e, &p) => {}
                _ => return Err(fail(OcError::denied("missing or invalid CSRF token"))),
            }
        }
        return Ok(Credentials {
            session: Some(session),
            agent_token: None,
        });
    }
    Ok(Credentials::default())
}

async fn run(st: &AppState, mut req: Request, headers: &HeaderMap, mutation: bool) -> Result<Envelope, ApiError> {
    req.credentials = credentials(st, headers, mutation)?;
    req.transport = "http".into();
    let svc = st.svc.clone();
    let env = tokio::task::spawn_blocking(move || svc.execute(req))
        .await
        .map_err(|_| fail(OcError::internal("request task panicked")))?;
    Ok(env)
}

fn respond(env: Envelope) -> Response {
    let status = match &env.error {
        None => StatusCode::OK,
        Some(e) => StatusCode::from_u16(e.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    };
    (status, Json(env)).into_response()
}

#[derive(Deserialize, Default)]
struct OpBody {
    #[serde(default)]
    params: Value,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    approval_id: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
}

/// Generic operation endpoint: everything the CLI can do.
async fn op_handler(State(st): State<AppState>, Path(op): Path<String>, headers: HeaderMap, body: Option<Json<OpBody>>) -> Result<Response, ApiError> {
    let b = body.map(|j| j.0).unwrap_or_default();
    let spec = openclaim_core::ops::find(&op).ok_or_else(|| fail(OcError::invalid(format!("unknown operation '{op}'"))))?;
    let mut req = Request::new(&op, b.params);
    req.dry_run = b.dry_run;
    req.idempotency_key = b.idempotency_key;
    req.approval_id = b.approval_id;
    req.request_id = b.request_id;
    req.non_interactive = true;
    Ok(respond(run(&st, req, &headers, spec.effect.is_mutation()).await?))
}

fn query_to_params(q: HashMap<String, String>) -> Value {
    let mut m = Map::new();
    for (k, v) in q {
        let val = match v.as_str() {
            "true" => json!(true),
            "false" => json!(false),
            s if s.parse::<i64>().is_ok() && !s.starts_with('0') => json!(s.parse::<i64>().unwrap()),
            s if s.contains(',') && k == "status" => json!(s.split(',').map(str::to_string).collect::<Vec<_>>()),
            s => json!(s),
        };
        m.insert(k, val);
    }
    Value::Object(m)
}

macro_rules! get_op {
    ($name:ident, $op:literal) => {
        async fn $name(State(st): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Response, ApiError> {
            Ok(respond(run(&st, Request::new($op, query_to_params(q)), &headers, false).await?))
        }
    };
}

macro_rules! get_op_id {
    ($name:ident, $op:literal, $key:literal) => {
        async fn $name(
            State(st): State<AppState>,
            headers: HeaderMap,
            Path(id): Path<String>,
            Query(q): Query<HashMap<String, String>>,
        ) -> Result<Response, ApiError> {
            let mut p = query_to_params(q);
            p[$key] = json!(id);
            if let Some(inc) = p.get("include").and_then(|v| v.as_str()).map(str::to_string) {
                p["include"] = json!(inc.split(',').map(str::to_string).collect::<Vec<_>>());
            }
            Ok(respond(run(&st, Request::new($op, p), &headers, false).await?))
        }
    };
}

macro_rules! post_op {
    ($name:ident, $op:literal) => {
        async fn $name(State(st): State<AppState>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<Response, ApiError> {
            let params = body.map(|j| j.0).unwrap_or(Value::Null);
            Ok(respond(run(&st, Request::new($op, params), &headers, true).await?))
        }
    };
}

macro_rules! post_op_id {
    ($name:ident, $op:literal, $key:literal) => {
        async fn $name(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<Response, ApiError> {
            let mut params = body.map(|j| j.0).unwrap_or_else(|| json!({}));
            params[$key] = json!(id);
            Ok(respond(run(&st, Request::new($op, params), &headers, true).await?))
        }
    };
}

get_op!(opportunities_list, "opportunities.list");
get_op_id!(opportunities_show, "opportunities.show", "id");
get_op_id!(opportunities_provenance, "opportunities.sources", "id");
post_op!(matches_evaluate, "match.evaluate");
get_op!(people_list, "people.list");
post_op!(claims_create, "claims.init");
post_op_id!(claims_patch, "claims.update", "claim_id");
post_op!(documents_import, "documents.import");
get_op!(sources_health, "sources.health");
post_op!(sources_refresh, "sources.refresh");
get_op!(alerts_list, "alerts.list");
post_op!(exports, "data.export");
post_op!(user_data_delete, "data.wipe");
get_op!(jobs_list, "jobs.list");
post_op_id!(jobs_cancel, "jobs.cancel", "job_id");
get_op!(agents_sessions, "agents.sessions");
get_op!(agents_approvals, "approvals.list");
post_op_id!(agents_revoke, "agents.revoke", "id");

async fn approval_decision(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let op = match body["decision"].as_str() {
        Some("approve") => "approvals.approve",
        Some("deny") => "approvals.deny",
        _ => return Err(fail(OcError::invalid("decision must be approve or deny"))),
    };
    let digest = body["digest"].as_str().map(str::to_string);
    if let Some(d) = digest {
        // Digest binding: the GUI sends the digest it displayed; mismatch means the request changed.
        let current = run(&st, Request::new("approvals.show", json!({"id": id})), &headers, false).await?;
        let shown = current.data.and_then(|v| v["action_digest"].as_str().map(str::to_string));
        if shown.as_deref() != Some(&d) {
            return Err(fail(OcError::conflict("the approval changed since it was displayed")));
        }
    }
    Ok(respond(
        run(&st, Request::new(op, json!({"id": id, "note": body["note"]})), &headers, true).await?,
    ))
}

#[derive(Deserialize)]
struct LoginBody {
    passphrase: String,
}

async fn login(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<LoginBody>) -> Result<Response, ApiError> {
    let mut req = Request::new("owner.unlock", json!({"passphrase": body.passphrase}));
    req.transport = "http".into();
    let svc = st.svc.clone();
    let env = tokio::task::spawn_blocking(move || svc.execute(req))
        .await
        .map_err(|_| fail(OcError::internal("task panicked")))?;
    let _ = headers;
    let Some(data) = env.data.clone() else { return Ok(respond(env)) };
    let session = data["session"].as_str().unwrap_or("").to_string();
    let csrf = openclaim_security::tokens::new_token("occ");
    st.csrf
        .lock()
        .map_err(|_| fail(OcError::internal("lock")))?
        .insert(session.clone(), csrf.clone());
    let mut resp = respond(Envelope::success(
        env.request_id,
        env.trace_id,
        json!({"csrf": csrf, "expires_at": data["expires_at"]}),
    ));
    let cookie = format!("{COOKIE}={session}; Path=/; HttpOnly; SameSite=Strict");
    resp.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    Ok(resp)
}

async fn logout(State(st): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(session) = cookie_value(&headers) {
        st.csrf.lock().map_err(|_| fail(OcError::internal("lock")))?.remove(&session);
        let _ = run(&st, Request::new("owner.lock", Value::Null), &headers, false).await;
    }
    let mut resp = respond(Envelope::success(
        openclaim_core::new_request_id(),
        openclaim_core::new_request_id(),
        json!({"locked": true}),
    ));
    resp.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("oc_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    Ok(resp)
}

async fn me(State(st): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    Ok(respond(run(&st, Request::new("owner.status", Value::Null), &headers, false).await?))
}

async fn health(State(st): State<AppState>) -> Response {
    Json(json!({"ok": true, "product": openclaim_domain::PRODUCT_NAME, "version": openclaim_domain::VERSION, "instance_id": st.svc.inner.instance_id}))
        .into_response()
}

async fn events(State(st): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let creds = credentials(&st, &headers, false)?;
    let probe = {
        let svc = st.svc.clone();
        let mut req = Request::new("events.poll", json!({"after_seq": i64::MAX - 1, "limit": 1}));
        req.credentials = creds;
        tokio::task::spawn_blocking(move || svc.execute(req))
            .await
            .map_err(|_| fail(OcError::internal("task")))?
    };
    if let Some(e) = probe.error {
        return Err(fail(e));
    }
    let rx = st.svc.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|r| r.ok()).map(|ev| {
        Ok::<_, std::convert::Infallible>(
            sse::Event::default()
                .event(ev.topic.clone())
                .id(ev.seq.to_string())
                .data(serde_json::to_string(&ev).unwrap_or_default()),
        )
    });
    Ok(sse::Sse::new(stream).keep_alive(sse::KeepAlive::default()).into_response())
}

use tokio_stream::StreamExt;

async fn spa(State(st): State<AppState>, uri: axum::http::Uri) -> Response {
    let Some(dir) = &st.gui_dir else {
        return (StatusCode::NOT_FOUND, "GUI assets are not installed; the CLI and API still work").into_response();
    };
    let rel = uri.path().trim_start_matches('/');
    let candidate = if rel.is_empty() { dir.join("index.html") } else { dir.join(rel) };
    let safe = candidate
        .canonicalize()
        .ok()
        .filter(|c| c.starts_with(dir.canonicalize().unwrap_or_else(|_| dir.clone())));
    let file = match safe {
        Some(f) if f.is_file() => f,
        _ => dir.join("index.html"),
    };
    match tokio::fs::read(&file).await {
        Ok(bytes) => {
            let mime = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript",
                Some("css") => "text/css",
                Some("json") => "application/json",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("ico") => "image/x-icon",
                Some("woff2") => "font/woff2",
                _ => "application/octet-stream",
            };
            ([(header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/events", get(events))
        .route("/ops/{op}", post(op_handler))
        .route("/opportunities", get(opportunities_list))
        .route("/opportunities/{id}", get(opportunities_show))
        .route("/opportunities/{id}/provenance", get(opportunities_provenance))
        .route("/matches/evaluate", post(matches_evaluate))
        .route("/people", get(people_list))
        .route("/claims", post(claims_create))
        .route("/claims/{id}", axum::routing::patch(claims_patch))
        .route("/documents/import", post(documents_import))
        .route("/sources/health", get(sources_health))
        .route("/sources/refresh", post(sources_refresh))
        .route("/alerts", get(alerts_list))
        .route("/exports", post(exports))
        .route("/user-data", axum::routing::delete(user_data_delete))
        .route("/jobs", get(jobs_list))
        .route("/jobs/{id}/cancel", post(jobs_cancel))
        .route("/agents/sessions", get(agents_sessions))
        .route("/agents/approvals", get(agents_approvals))
        .route("/agents/approvals/{id}/decision", post(approval_decision))
        .route("/agents/revoke/{id}", post(agents_revoke));
    Router::new()
        .nest("/api/v1", api)
        .fallback(get(spa))
        .layer(axum::extract::DefaultBodyLimit::max(12 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

pub async fn serve(state: AppState, shutdown: impl std::future::Future<Output = ()> + Send + 'static) -> Result<u16, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], state.port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("bind {addr}: {e}"))?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(state.port);
    let app = router(AppState { port, ..state });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).with_graceful_shutdown(shutdown).await;
    });
    Ok(port)
}
