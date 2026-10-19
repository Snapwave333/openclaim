//! Local IPC transport. One JSON `Request` per line in, one JSON `Envelope`
//! per line out. The OS restricts the endpoint to the owning user; request
//! credentials (owner session / agent token) decide what the caller may do.

use openclaim_core::{Request, Service};
use openclaim_domain::{Envelope, OcError};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub name: String,
    pub socket_path: PathBuf,
}

impl Endpoint {
    pub fn for_paths(paths: &openclaim_core::Paths) -> Endpoint {
        let name = paths.ipc_name();
        Endpoint { socket_path: paths.data_dir.join(format!("{name}.sock")), name }
    }

    #[cfg(windows)]
    pub fn pipe_path(&self) -> String {
        format!(r"\\.\pipe\{}", self.name)
    }

    pub fn display(&self) -> String {
        #[cfg(windows)]
        {
            self.pipe_path()
        }
        #[cfg(not(windows))]
        {
            self.socket_path.to_string_lossy().to_string()
        }
    }
}

async fn handle<S>(svc: Service, stream: S)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r).take(MAX_REQUEST_BYTES as u64);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => return,
            Ok(_) => {}
            Err(_) => return,
        }
        reader.set_limit(MAX_REQUEST_BYTES as u64);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(req) => {
                let svc = svc.clone();
                tokio::task::spawn_blocking(move || svc.execute(req)).await.unwrap_or_else(|_| {
                    Envelope::failure(String::new(), String::new(), OcError::internal("request task panicked"))
                })
            }
            Err(e) => Envelope::failure(String::new(), String::new(), OcError::invalid(format!("malformed request: {e}"))),
        };
        let mut out = serde_json::to_string(&response).unwrap_or_else(|_| "{\"ok\":false}".into());
        out.push('\n');
        if w.write_all(out.as_bytes()).await.is_err() {
            return;
        }
    }
}

/// Runs the IPC server until `shutdown` resolves. Errors if another daemon owns the endpoint.
pub async fn serve(svc: Service, endpoint: Endpoint, shutdown: impl std::future::Future<Output = ()>) -> Result<(), String> {
    #[cfg(windows)]
    {
        serve_windows(svc, endpoint, shutdown).await
    }
    #[cfg(not(windows))]
    {
        serve_unix(svc, endpoint, shutdown).await
    }
}

#[cfg(windows)]
async fn serve_windows(svc: Service, endpoint: Endpoint, shutdown: impl std::future::Future<Output = ()>) -> Result<(), String> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let path = endpoint.pipe_path();
    let sd = win::owner_only_descriptor()?;
    let make = |first: bool| -> Result<tokio::net::windows::named_pipe::NamedPipeServer, String> {
        let mut opts = ServerOptions::new();
        opts.first_pipe_instance(first).reject_remote_clients(true);
        let mut attrs = win::security_attributes(&sd);
        // SAFETY: `attrs` points at a valid SECURITY_ATTRIBUTES whose descriptor outlives the call.
        unsafe { opts.create_with_security_attributes_raw(&path, &mut attrs as *mut _ as *mut _) }.map_err(|e| {
            if first { format!("another openclaimd already owns {path} ({e})") } else { e.to_string() }
        })
    };
    let mut server = make(true)?;
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            res = server.connect() => {
                res.map_err(|e| e.to_string())?;
                let connected = server;
                server = make(false)?;
                let svc = svc.clone();
                tokio::spawn(async move { handle(svc, connected).await });
            }
        }
    }
}

#[cfg(not(windows))]
async fn serve_unix(svc: Service, endpoint: Endpoint, shutdown: impl std::future::Future<Output = ()>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let path = &endpoint.socket_path;
    if path.exists() {
        // Refuse to steal a live socket; remove a dead one.
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return Err(format!("another openclaimd already owns {}", path.display()));
        }
        let _ = std::fs::remove_file(path);
    }
    let listener = tokio::net::UnixListener::bind(path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => { let _ = std::fs::remove_file(path); return Ok(()) }
            res = listener.accept() => {
                let (stream, _) = res.map_err(|e| e.to_string())?;
                let svc = svc.clone();
                tokio::spawn(async move { handle(svc, stream).await });
            }
        }
    }
}

/// Client side: one request, one envelope.
pub async fn call(endpoint: &Endpoint, req: &Request, timeout: std::time::Duration) -> Result<Envelope, OcError> {
    let fut = async {
        #[cfg(windows)]
        let stream = {
            use tokio::net::windows::named_pipe::ClientOptions;
            let path = endpoint.pipe_path();
            let mut attempts = 0;
            loop {
                match ClientOptions::new().open(&path) {
                    Ok(s) => break s,
                    Err(e) if e.raw_os_error() == Some(231) && attempts < 20 => {
                        // ERROR_PIPE_BUSY: every instance is taken; wait for a free one.
                        attempts += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    Err(e) => return Err(OcError::unavailable(format!("daemon not reachable at {path}: {e}")).with_next_action("run `openclaim daemon ensure`")),
                }
            }
        };
        #[cfg(not(windows))]
        let stream = tokio::net::UnixStream::connect(&endpoint.socket_path)
            .await
            .map_err(|e| OcError::unavailable(format!("daemon not reachable at {}: {e}", endpoint.socket_path.display())).with_next_action("run `openclaim daemon ensure`"))?;
        let (r, mut w) = tokio::io::split(stream);
        let mut line = serde_json::to_string(req).map_err(|e| OcError::internal(e.to_string()))?;
        line.push('\n');
        w.write_all(line.as_bytes()).await.map_err(|e| OcError::unavailable(format!("write to daemon failed: {e}")))?;
        let mut reader = BufReader::new(r).take(MAX_REQUEST_BYTES as u64 * 4);
        let mut out = String::new();
        reader.read_line(&mut out).await.map_err(|e| OcError::unavailable(format!("read from daemon failed: {e}")))?;
        if out.trim().is_empty() {
            return Err(OcError::unavailable("daemon closed the connection without answering"));
        }
        serde_json::from_str::<Envelope>(out.trim()).map_err(|e| OcError::internal(format!("malformed daemon reply: {e}")))
    };
    match tokio::time::timeout(timeout, fut).await {
        Ok(r) => r,
        Err(_) => Err(OcError::new(openclaim_domain::ErrorCode::Timeout, format!("no reply within {}s; a durable job may still continue", timeout.as_secs())).with_next_action("check `openclaim jobs list`")),
    }
}

/// Is a daemon answering on this endpoint?
pub async fn ping(endpoint: &Endpoint) -> bool {
    call(endpoint, &Request::new("schema.version", serde_json::Value::Null), std::time::Duration::from_secs(3)).await.map(|e| e.ok).unwrap_or(false)
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    /// Owned security descriptor granting access only to the object owner (the daemon's user) and SYSTEM.
    pub struct Descriptor(PSECURITY_DESCRIPTOR);

    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW with LocalAlloc.
            unsafe { LocalFree(self.0 as HLOCAL) };
        }
    }

    // SAFETY: the descriptor is immutable after creation and only read by CreateNamedPipe.
    unsafe impl Send for Descriptor {}
    unsafe impl Sync for Descriptor {}

    pub fn owner_only_descriptor() -> Result<Descriptor, String> {
        // D: DACL, P: protected (no inheritance); allow GENERIC_ALL to OWNER RIGHTS and SYSTEM only.
        let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)".encode_utf16().chain(std::iter::once(0)).collect();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: sddl is NUL-terminated; sd receives a LocalAlloc'd descriptor on success.
        let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) };
        if ok == 0 || sd.is_null() {
            return Err(format!("could not build pipe security descriptor: {}", std::io::Error::last_os_error()));
        }
        Ok(Descriptor(sd))
    }

    pub fn security_attributes(d: &Descriptor) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: d.0, bInheritHandle: 0 }
    }
}
