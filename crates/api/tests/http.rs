//! Localhost boundary tests: Host/Origin checks, CSRF, cookie auth, agent Bearer, SSE auth, headers.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use openclaim_api::{AppState, CSRF_HEADER, router};
use openclaim_core::{Paths, Service};
use serde_json::{Value, json};
use tower::ServiceExt;

const PORT: u16 = 7865;

fn app() -> (axum::Router, Service, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Paths::from_dir(dir.path().to_path_buf());
    paths.in_memory = true;
    let svc = Service::open(paths).unwrap();
    svc.execute(openclaim_core::Request::new("owner.init", json!({"passphrase": "correct horse battery"})));
    let state = AppState {
        svc: svc.clone(),
        port: PORT,
        gui_dir: None,
        csrf: Default::default(),
    };
    (router(state), svc, dir)
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Vec<(String, String)>, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).to_string()));
    (status, headers, json)
}

fn req(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder().method(method).uri(path).header(header::HOST, format!("127.0.0.1:{PORT}"))
}

fn header_of(h: &[(String, String)], name: &str) -> Option<String> {
    h.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
}

#[tokio::test]
async fn host_and_origin_are_enforced() {
    let (app, _svc, _d) = app();
    let (s, _, _) = send(
        &app,
        Request::builder()
            .uri("/api/v1/health")
            .header(header::HOST, "evil.example")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "DNS rebinding: foreign Host is refused");
    let (s, _, _) = send(
        &app,
        req("GET", "/api/v1/health")
            .header(header::ORIGIN, "https://evil.example")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "foreign Origin is refused");
    let (s, h, body) = send(
        &app,
        req("GET", "/api/v1/health")
            .header(header::ORIGIN, format!("http://127.0.0.1:{PORT}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(header_of(&h, "x-frame-options").as_deref(), Some("DENY"));
    assert!(header_of(&h, "content-security-policy").unwrap().contains("frame-ancestors 'none'"));
    assert!(header_of(&h, "access-control-allow-origin").is_none(), "no CORS for the local GUI origin");
}

#[tokio::test]
async fn login_sets_cookie_and_mutations_need_csrf() {
    let (app, _svc, _d) = app();
    let (s, _, _) = send(
        &app,
        req("POST", "/api/v1/ops/people.create")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"params":{"display_name":"x"}}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "anonymous cannot mutate");
    let (s, h, body) = send(
        &app,
        req("POST", "/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"passphrase":"correct horse battery"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let cookie = header_of(&h, "set-cookie").unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    let session = cookie.split(';').next().unwrap().to_string();
    let csrf = body["data"]["csrf"].as_str().unwrap().to_string();
    // Cookie without CSRF header: refused.
    let (s, _, _) = send(
        &app,
        req("POST", "/api/v1/ops/people.create")
            .header(header::COOKIE, &session)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"params":{"display_name":"x"}}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // Cookie + CSRF: allowed.
    let (s, _, body) = send(
        &app,
        req("POST", "/api/v1/ops/people.create")
            .header(header::COOKIE, &session)
            .header(CSRF_HEADER, &csrf)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"params":{"display_name":"Ann"}}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["display_name"], "Ann");
    // Reads with the cookie work without CSRF; /auth/me reports the owner.
    let (s, _, body) = send(
        &app,
        req("GET", "/api/v1/auth/me").header(header::COOKIE, &session).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["data"]["unlocked"], true);
    let (s, _, body) = send(&app, req("GET", "/api/v1/people").header(header::COOKIE, &session).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);
    // Reload: the cookie alone re-issues a CSRF token; without a cookie it is refused.
    let (s, _, body) = send(
        &app,
        req("GET", "/api/v1/auth/csrf").header(header::COOKIE, &session).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(body["data"]["csrf"].as_str().unwrap().starts_with("occ_"));
    let (s, _, _) = send(&app, req("GET", "/api/v1/auth/csrf").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // Bad login.
    let (s, _, _) = send(
        &app,
        req("POST", "/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"passphrase":"nope"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn agent_bearer_tokens_and_extension_cors() {
    let (app, svc, _d) = app();
    let unlock = svc.execute(openclaim_core::Request::new("owner.unlock", json!({"passphrase": "correct horse battery"})));
    let session = unlock.data.unwrap()["session"].as_str().unwrap().to_string();
    let reg = svc.execute(
        openclaim_core::Request::new(
            "agents.register",
            json!({"name": "ext", "kind": "extension", "scopes": ["extension.prefill", "sources.read"], "allowed_origin": "chrome-extension://abcdef"}),
        )
        .with_session(&session),
    );
    let token = reg.data.unwrap()["token"].as_str().unwrap().to_string();
    // Preflight from the paired extension origin succeeds with CORS headers; others are refused.
    let (s, h, _) = send(
        &app,
        req("OPTIONS", "/api/v1/ops/sources.list")
            .header(header::ORIGIN, "chrome-extension://abcdef")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(header_of(&h, "access-control-allow-origin").as_deref(), Some("chrome-extension://abcdef"));
    let (s, _, _) = send(
        &app,
        req("OPTIONS", "/api/v1/ops/sources.list")
            .header(header::ORIGIN, "chrome-extension://other")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // Bearer token works for its scopes and is denied elsewhere (HTTP 403 mirrors PERMISSION_DENIED).
    let (s, _, body) = send(
        &app,
        req("POST", "/api/v1/ops/sources.list")
            .header(header::ORIGIN, "chrome-extension://abcdef")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(body["data"]["items"][0]["allowed_hosts"].is_array());
    let (s, _, body) = send(
        &app,
        req("POST", "/api/v1/ops/people.list")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "PERMISSION_DENIED");
    // Unknown op → 400; unknown token → 403.
    let (s, _, _) = send(
        &app,
        req("POST", "/api/v1/ops/no.such")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = send(
        &app,
        req("POST", "/api/v1/ops/sources.list")
            .header(header::AUTHORIZATION, "Bearer oca_bogus")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sse_requires_a_reader_and_spa_fallback_is_safe() {
    let (app, _svc, _d) = app();
    // Anonymous has no jobs.read → SSE refused.
    let (s, _, _) = send(&app, req("GET", "/api/v1/events").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // No GUI dir: fallback is a plain 404, never a directory listing or path traversal.
    let (s, _, body) = send(&app, req("GET", "/../../Cargo.toml").body(Body::empty()).unwrap()).await;
    assert!(s == StatusCode::NOT_FOUND || s == StatusCode::BAD_REQUEST, "{s} {body}");
}
