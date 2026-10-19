//! `openclaimd`: owns the database, runs durable jobs and schedules, serves the
//! local IPC endpoint and (optionally) the localhost REST/GUI. No model, no
//! inference runtime, no outbound traffic beyond permitted free sources.

use clap::Parser;
use openclaim_core::{Paths, Service};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "openclaimd", version = openclaim_domain::VERSION, about = "OpenClaim headless daemon")]
struct Args {
    /// Data directory (default: per-user OpenClaim folder, or OPENCLAIM_HOME)
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Localhost port for the REST API and GUI (0 = pick a free port)
    #[arg(long)]
    port: Option<u16>,
    /// Run without the HTTP API (IPC only)
    #[arg(long)]
    no_http: bool,
    /// Directory with the built Angular GUI (default: OPENCLAIM_GUI_DIR or ./gui next to the binary)
    #[arg(long)]
    gui_dir: Option<PathBuf>,
    /// Log to stderr as well as the log file
    #[arg(long)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    let code = rt.block_on(run(args));
    std::process::exit(code);
}

async fn run(args: Args) -> i32 {
    let paths = Paths::resolve(args.data_dir.clone());
    let _ = std::fs::create_dir_all(&paths.log_dir);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log_dir.join("openclaimd.log"))
        .ok();
    let filter = tracing_subscriber::EnvFilter::try_from_env("OPENCLAIM_LOG").unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    match (log_file, args.verbose) {
        (Some(f), false) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(Arc::new(f))
            .with_ansi(false)
            .init(),
        (Some(f), true) => {
            let w = Arc::new(f);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(move || Tee(w.clone()))
                .with_ansi(false)
                .init();
        }
        (None, _) => tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init(),
    }

    let svc = match Service::open(paths.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("openclaimd: {}", e.message);
            tracing::error!("open service: {}", e.message);
            return 11;
        }
    };
    tracing::info!(version = openclaim_domain::VERSION, data_dir = %paths.data_dir.display(), "starting");

    let endpoint = openclaim_ipc::Endpoint::for_paths(&paths);
    let shutdown_ipc = {
        let svc = svc.clone();
        async move { svc.inner.shutdown.notified().await }
    };
    let ipc_task = {
        let svc = svc.clone();
        let ep = endpoint.clone();
        tokio::spawn(async move { openclaim_ipc::serve(svc, ep, shutdown_ipc).await })
    };
    // Give the IPC server a moment to claim the endpoint so a second instance exits early.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    if ipc_task.is_finished() {
        match ipc_task.await {
            Ok(Err(e)) => eprintln!("openclaimd: {e}"),
            _ => eprintln!("openclaimd: IPC server stopped unexpectedly"),
        }
        return 7;
    }

    let mut http_port = None;
    if !args.no_http {
        let port = args.port.unwrap_or_else(|| {
            svc.inner
                .db
                .lock()
                .ok()
                .map(|g| openclaim_core::modules::config::get_u64(&g.conn, "gui.port", 7865) as u16)
                .unwrap_or(7865)
        });
        let gui_dir = args
            .gui_dir
            .or_else(|| std::env::var_os("OPENCLAIM_GUI_DIR").map(PathBuf::from))
            .or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("gui"))))
            .filter(|p| p.join("index.html").is_file());
        let state = openclaim_api::AppState {
            svc: svc.clone(),
            port,
            gui_dir: gui_dir.clone(),
            csrf: Default::default(),
        };
        let shutdown = {
            let svc = svc.clone();
            async move { svc.inner.shutdown.notified().await }
        };
        match openclaim_api::serve(state, shutdown).await {
            Ok(p) => {
                http_port = Some(p);
                tracing::info!(port = p, gui = ?gui_dir, "http api listening on 127.0.0.1");
            }
            Err(e) => tracing::warn!("http api disabled: {e}"),
        }
    }

    let info = serde_json::json!({
        "pid": std::process::id(),
        "ipc": endpoint.display(),
        "http_port": http_port,
        "started_at": openclaim_domain::dates::now_rfc3339(),
        "version": openclaim_domain::VERSION,
    });
    let _ = std::fs::write(paths.owner_token_hint(), info.to_string());

    let jobs = {
        let svc = svc.clone();
        tokio::spawn(async move { svc.run_jobs().await })
    };

    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    tokio::select! {
        _ = ctrl_c => { tracing::info!("ctrl-c: stopping"); }
        _ = svc.inner.shutdown.notified() => { tracing::info!("stop requested"); }
    }
    svc.inner.stop_requested.store(true, std::sync::atomic::Ordering::SeqCst);
    svc.inner.shutdown.notify_waiters();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), jobs).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), ipc_task).await;
    let _ = std::fs::remove_file(paths.owner_token_hint());
    tracing::info!("stopped");
    0
}

struct Tee(Arc<std::fs::File>);

impl std::io::Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        (&*self.0).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}
