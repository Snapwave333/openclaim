//! Concurrency and restart recovery (MDR §14.10, §22.2 items 8-9).

use openclaim_core::{Paths, Request, Service};
use serde_json::json;

fn file_backed() -> (Service, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_dir(dir.path().join("data"));
    let svc = Service::open(paths).unwrap();
    svc.execute(Request::new("owner.init", json!({"passphrase": "correct horse battery"})));
    (svc, dir)
}

fn session(svc: &Service) -> String {
    svc.execute(Request::new("owner.unlock", json!({"passphrase": "correct horse battery"})))
        .data
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn parallel_writers_do_not_lose_updates() {
    let (svc, _d) = file_backed();
    let s = session(&svc);
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let svc = svc.clone();
            let s = s.clone();
            std::thread::spawn(move || {
                for j in 0..10 {
                    let env = svc.execute(Request::new("people.create", json!({"display_name": format!("p{i}-{j}")})).with_session(&s));
                    assert!(env.ok, "{:?}", env.error);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let n = svc.execute(Request::new("people.list", json!({"limit": 500})).with_session(&s)).data.unwrap()["items"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(n, 80);
    // Optimistic concurrency: a stale expected_version is refused.
    let p = svc.execute(Request::new("people.list", json!({"limit": 1})).with_session(&s)).data.unwrap()["items"][0].clone();
    let id = p["id"].as_str().unwrap();
    let ok = svc.execute(Request::new("people.update", json!({"id": id, "display_name": "renamed", "expected_version": p["version"]})).with_session(&s));
    assert!(ok.ok);
    let stale = svc.execute(Request::new("people.update", json!({"id": id, "display_name": "again", "expected_version": p["version"]})).with_session(&s));
    assert_eq!(stale.error.unwrap().code, openclaim_domain::ErrorCode::Conflict);
}

#[tokio::test]
async fn jobs_survive_a_daemon_restart() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_dir(dir.path().join("data"));
    let job_id;
    {
        let svc = Service::open(paths.clone()).unwrap();
        svc.execute(Request::new("owner.init", json!({"passphrase": "correct horse battery"})));
        let s = session(&svc);
        let env = svc.execute(Request::new("jobs.enqueue", json!({"kind": "integrity_check"})).with_session(&s));
        job_id = env.data.unwrap()["job_id"].as_str().unwrap().to_string();
        // Simulate a crash mid-run: the job holds an expired lease when the process dies.
        let guard = svc.inner.db.lock().unwrap();
        guard
            .conn
            .execute(
                "UPDATE jobs SET state = 'running', lease_owner = 'dead', lease_expires_at = '2000-01-01T00:00:00Z' WHERE id = ?1",
                [&job_id],
            )
            .unwrap();
    }
    let svc = Service::open(paths).unwrap();
    assert_eq!(svc.recover_jobs().unwrap(), 1);
    let s = session(&svc);
    let shown = svc.execute(Request::new("jobs.show", json!({"job_id": job_id})).with_session(&s)).data.unwrap();
    assert_eq!(shown["state"], "queued");
    svc.drain_jobs().await;
    let shown = svc.execute(Request::new("jobs.show", json!({"job_id": job_id})).with_session(&s)).data.unwrap();
    assert_eq!(shown["state"], "succeeded", "{shown}");
    // Sessions are memory-only: the pre-restart session is gone, persisted state is intact.
    let people = svc.execute(Request::new("audit.verify", json!({})).with_session(&s)).data.unwrap();
    assert_eq!(people["intact"], true);
}
