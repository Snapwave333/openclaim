use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub vault_dir: PathBuf,
    pub log_dir: PathBuf,
    pub in_memory: bool,
}

impl Paths {
    /// `OPENCLAIM_HOME` overrides the default per-user location.
    pub fn resolve(override_dir: Option<PathBuf>) -> Paths {
        let data_dir = override_dir
            .or_else(|| std::env::var_os("OPENCLAIM_HOME").map(PathBuf::from))
            .unwrap_or_else(default_dir);
        Paths::from_dir(data_dir)
    }

    pub fn from_dir(data_dir: PathBuf) -> Paths {
        Paths {
            db_path: data_dir.join("openclaim.sqlite"),
            vault_dir: data_dir.join("vault"),
            log_dir: data_dir.join("logs"),
            data_dir,
            in_memory: false,
        }
    }

    pub fn ephemeral() -> Paths {
        let dir = std::env::temp_dir().join(format!("openclaim-test-{}", openclaim_db::new_id()));
        let mut p = Paths::from_dir(dir);
        p.in_memory = true;
        p
    }

    pub fn ipc_name(&self) -> String {
        let digest = openclaim_security::digest::sha256_hex(self.data_dir.to_string_lossy().to_lowercase().as_bytes());
        format!("openclaim-{}", &digest[..16])
    }

    pub fn owner_token_hint(&self) -> PathBuf {
        self.data_dir.join("daemon.json")
    }
}

fn default_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("OpenClaim")
    } else if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("openclaim")
    } else {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join(".local/share/openclaim")
    }
}
