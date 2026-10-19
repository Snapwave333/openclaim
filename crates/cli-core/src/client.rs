//! Daemon client with explicit, configurable auto-start (MDR §16.2).

use crate::Globals;
use openclaim_core::{Paths, Request};
use openclaim_domain::{Envelope, OcError};
use openclaim_ipc::Endpoint;
use std::path::PathBuf;
use std::time::Duration;

pub struct Client {
    pub paths: Paths,
    pub endpoint: Endpoint,
    pub timeout: Duration,
    pub autostart: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct DaemonInfo {
    pub pid: u32,
    pub ipc: String,
    pub http_port: Option<u16>,
    pub started_at: String,
    pub version: String,
}

impl Client {
    pub fn new(g: &Globals) -> Client {
        let paths = Paths::resolve(g.data_dir.clone());
        Client { endpoint: Endpoint::for_paths(&paths), paths, timeout: Duration::from_secs(g.timeout_secs.max(1)), autostart: !g.no_autostart }
    }

    pub fn daemon_info(&self) -> Option<DaemonInfo> {
        let text = std::fs::read_to_string(self.paths.owner_token_hint()).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn daemon_binary() -> Result<PathBuf, OcError> {
        let exe = std::env::current_exe().map_err(|e| OcError::internal(e.to_string()))?;
        let name = if cfg!(windows) { "openclaimd.exe" } else { "openclaimd" };
        let sibling = exe.parent().map(|p| p.join(name)).filter(|p| p.exists());
        if let Some(s) = sibling {
            return Ok(s);
        }
        let on_path = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|c| c.is_file()));
        on_path.ok_or_else(|| OcError::unavailable("openclaimd was not found next to openclaim or on PATH").with_next_action("install openclaimd or start it yourself"))
    }

    /// Spawns openclaimd detached from this terminal.
    pub fn spawn_daemon(&self) -> Result<u32, OcError> {
        let bin = Self::daemon_binary()?;
        let mut cmd = std::process::Command::new(bin);
        cmd.arg("--data-dir").arg(&self.paths.data_dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let child = cmd.spawn().map_err(|e| OcError::unavailable(format!("could not start openclaimd: {e}")))?;
        Ok(child.id())
    }

    pub async fn ensure(&self) -> Result<bool, OcError> {
        if openclaim_ipc::ping(&self.endpoint).await {
            return Ok(false);
        }
        if !self.autostart {
            return Err(OcError::unavailable("daemon is not running and auto-start is disabled").with_next_action("run `openclaim daemon start`"));
        }
        self.spawn_daemon()?;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if openclaim_ipc::ping(&self.endpoint).await {
                return Ok(true);
            }
        }
        Err(OcError::unavailable("openclaimd did not answer within 15 seconds").with_next_action(format!("check {}", self.paths.log_dir.join("openclaimd.log").display())))
    }

    pub async fn call(&self, req: &Request) -> Envelope {
        let rid = req.request_id.clone().unwrap_or_default();
        match openclaim_ipc::call(&self.endpoint, req, self.timeout).await {
            Ok(env) => env,
            Err(e) if e.code == openclaim_domain::ErrorCode::Unavailable => match self.ensure().await {
                Ok(_) => match openclaim_ipc::call(&self.endpoint, req, self.timeout).await {
                    Ok(env) => env,
                    Err(e) => Envelope::failure(rid, String::new(), e),
                },
                Err(e2) => Envelope::failure(rid, String::new(), e2),
            },
            Err(e) => Envelope::failure(rid, String::new(), e),
        }
    }
}
