//! Application services shared by every surface. A single `Service` owns the
//! database; every operation runs through `Service::execute`, which resolves
//! the actor, enforces scopes and approvals, runs the handler inside one
//! transaction (rolled back for `--dry-run`), and records audit + command runs.

pub mod audit;
pub mod authz;
pub mod ctx;
pub mod dispatch;
pub mod events;
pub mod jobs;
pub mod modules;
pub mod ops;
pub mod paths;
pub mod sql;

use openclaim_db::Database;
use openclaim_domain::{OcError, OcResult};
use openclaim_security::vault::VaultKey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, broadcast};

pub use ctx::{Actor, Credentials, Request};
pub use dispatch::Response;
pub use paths::Paths;

pub fn new_request_id() -> String {
    openclaim_db::new_id()
}

#[derive(Debug, Clone)]
pub struct OwnerSession {
    pub id: String,
    pub expires_at: time::OffsetDateTime,
    pub transport: String,
}

pub struct Inner {
    pub db: Mutex<Database>,
    pub vault: VaultKey,
    pub paths: Paths,
    pub events: broadcast::Sender<events::Event>,
    pub job_wake: Notify,
    pub sessions: Mutex<HashMap<String, OwnerSession>>,
    pub unlock_attempts: Mutex<Vec<time::OffsetDateTime>>,
    pub started_at: time::OffsetDateTime,
    pub instance_id: String,
    pub stop_requested: std::sync::atomic::AtomicBool,
    pub shutdown: Notify,
}

#[derive(Clone)]
pub struct Service {
    pub inner: Arc<Inner>,
}

impl Service {
    pub fn open(paths: Paths) -> OcResult<Self> {
        std::fs::create_dir_all(&paths.data_dir).map_err(|e| OcError::internal(format!("create {}: {e}", paths.data_dir.display())))?;
        std::fs::create_dir_all(&paths.vault_dir).map_err(|e| OcError::internal(format!("create vault dir: {e}")))?;
        let vault = VaultKey::load_or_create(&paths.data_dir).map_err(|e| OcError::internal(e.to_string()))?;
        let db = if paths.in_memory {
            Database::open_in_memory()?
        } else {
            Database::open(&paths.db_path)?
        };
        let (events, _) = broadcast::channel(1024);
        let svc = Service {
            inner: Arc::new(Inner {
                db: Mutex::new(db),
                vault,
                paths,
                events,
                job_wake: Notify::new(),
                sessions: Mutex::new(HashMap::new()),
                unlock_attempts: Mutex::new(Vec::new()),
                started_at: time::OffsetDateTime::now_utc(),
                instance_id: openclaim_db::new_id(),
                stop_requested: std::sync::atomic::AtomicBool::new(false),
                shutdown: Notify::new(),
            }),
        };
        svc.bootstrap()?;
        Ok(svc)
    }

    /// Seeds bundled connectors and schedules on first run.
    fn bootstrap(&self) -> OcResult<()> {
        let mut guard = self.inner.db.lock().map_err(|_| OcError::internal("database lock poisoned"))?;
        let tx = guard.conn.transaction().map_err(openclaim_db::db_err)?;
        modules::sources::seed(&tx)?;
        jobs::seed_schedules(&tx)?;
        tx.commit().map_err(openclaim_db::db_err)?;
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<events::Event> {
        self.inner.events.subscribe()
    }

    pub fn vault(&self) -> &VaultKey {
        &self.inner.vault
    }
}
