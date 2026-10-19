use clap::ArgMatches;
use openclaim_cli_core::client::Client;
use openclaim_cli_core::output::{print_envelope, print_ndjson_line};
use openclaim_cli_core::{Globals, Output, build_command, envelope_from_local, globals, help_json, params_from, request_for, resolve, spec::CmdSpec};
use openclaim_domain::{Envelope, OcError};
use serde_json::{Value, json};
use std::io::IsTerminal;

fn main() {
    let code = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(run());
    std::process::exit(code);
}

async fn run() -> i32 {
    let mut cmd = build_command();
    let matches = match cmd.clone().try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            // clap prints help/version itself; usage errors map to INVALID_ARGUMENT.
            let is_help = matches!(e.kind(), clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion);
            let _ = e.print();
            return if is_help { 0 } else { 2 };
        }
    };
    let g = globals(&matches);
    if matches.subcommand_name() == Some("help-tree") {
        return print_envelope(
            &envelope_from_local(help_json(), &g),
            if g.output.is_machine() { g.output } else { Output::Json },
        );
    }
    let Some((spec, leaf)) = resolve(&matches) else {
        let _ = cmd.print_help();
        return 2;
    };
    let client = Client::new(&g);
    let env = match spec.op.strip_prefix("local:") {
        Some(local) => match local_command(local, spec, leaf, &g, &client).await {
            Ok(Some(env)) => env,
            Ok(None) => return 0,
            Err(e) => Envelope::failure(g.request_id.clone().unwrap_or_default(), String::new(), e),
        },
        None => match params_from(spec, leaf, &g) {
            Ok(params) => client.call(&request_for(spec.op, params, &g)).await,
            Err(e) => Envelope::failure(g.request_id.clone().unwrap_or_default(), String::new(), e),
        },
    };
    print_envelope(&env, g.output)
}

fn read_passphrase(prompt: &str, from_stdin: bool, g: &Globals) -> Result<String, OcError> {
    if from_stdin || !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).map_err(|e| OcError::invalid(e.to_string()))?;
        let s = s.trim_end_matches(['\r', '\n']).to_string();
        if s.is_empty() {
            return Err(OcError::invalid("no passphrase on stdin"));
        }
        return Ok(s);
    }
    if g.non_interactive {
        return Err(OcError::invalid("a passphrase prompt is needed; use --passphrase-stdin"));
    }
    rpassword::prompt_password(prompt).map_err(|e| OcError::invalid(e.to_string()))
}

async fn local_command(name: &str, spec: &CmdSpec, leaf: &ArgMatches, g: &Globals, client: &Client) -> Result<Option<Envelope>, OcError> {
    match name {
        "init" => {
            let p = read_passphrase("Choose an owner passphrase: ", leaf.get_flag("passphrase-stdin"), g)?;
            if std::io::stdin().is_terminal() && !leaf.get_flag("passphrase-stdin") {
                let again = read_passphrase("Repeat it: ", false, g)?;
                if again != p {
                    return Err(OcError::invalid("passphrases do not match"));
                }
            }
            Ok(Some(client.call(&request_for("owner.init", json!({"passphrase": p}), g)).await))
        }
        "unlock" => {
            let p = read_passphrase("Owner passphrase: ", leaf.get_flag("passphrase-stdin"), g)?;
            let ttl = leaf.get_one::<i64>("ttl").copied();
            let env = client.call(&request_for("owner.unlock", json!({"passphrase": p, "ttl_minutes": ttl}), g)).await;
            if env.ok && !g.output.is_machine() {
                let token = env.data.as_ref().and_then(|d| d["session"].as_str()).unwrap_or("").to_string();
                eprintln!("Session created. Export it for this shell:");
                if cfg!(windows) {
                    println!("$env:OPENCLAIM_SESSION=\"{token}\"");
                } else {
                    println!("export OPENCLAIM_SESSION=\"{token}\"");
                }
                return Ok(None);
            }
            Ok(Some(env))
        }
        "help_json" => Ok(Some(envelope_from_local(help_json(), g))),
        "daemon_ensure" => {
            let started = client.ensure().await?;
            let status = client.call(&request_for("daemon.status", Value::Null, g)).await;
            let mut env = status;
            if let Some(d) = env.data.as_mut() {
                d["started_now"] = json!(started);
                d["ipc"] = json!(client.endpoint.display());
            }
            Ok(Some(env))
        }
        "daemon_start" => {
            if leaf.get_flag("foreground") {
                let bin = Client::daemon_binary()?;
                let status = std::process::Command::new(bin)
                    .arg("--data-dir")
                    .arg(&client.paths.data_dir)
                    .status()
                    .map_err(|e| OcError::unavailable(e.to_string()))?;
                return Ok(Some(envelope_from_local(json!({"exited": status.code()}), g)));
            }
            if openclaim_ipc_ping(client).await {
                return Ok(Some(envelope_from_local(
                    json!({"running": true, "started_now": false, "ipc": client.endpoint.display()}),
                    g,
                )));
            }
            let pid = client.spawn_daemon()?;
            let ok = wait_for_daemon(client).await;
            Ok(Some(envelope_from_local(
                json!({"running": ok, "started_now": ok, "pid": pid, "ipc": client.endpoint.display()}),
                g,
            )))
        }
        "daemon_stop" => {
            let env = client.call(&request_for("daemon.stop", Value::Null, g)).await;
            Ok(Some(env))
        }
        "daemon_restart" => {
            let _ = client.call(&request_for("daemon.stop", Value::Null, g)).await;
            for _ in 0..40 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if !openclaim_ipc_ping(client).await {
                    break;
                }
            }
            let pid = client.spawn_daemon()?;
            let ok = wait_for_daemon(client).await;
            Ok(Some(envelope_from_local(json!({"running": ok, "pid": pid}), g)))
        }
        "daemon_status" => {
            let info = client.daemon_info();
            let reachable = openclaim_ipc_ping(client).await;
            let mut data =
                json!({"reachable": reachable, "ipc": client.endpoint.display(), "data_dir": client.paths.data_dir.to_string_lossy(), "daemon_file": info});
            if reachable {
                let env = client.call(&request_for("daemon.status", Value::Null, g)).await;
                data["daemon"] = env.data.unwrap_or(Value::Null);
            }
            Ok(Some(envelope_from_local(data, g)))
        }
        "watch_events" => {
            let secs = leaf.get_one::<i64>("seconds").copied().unwrap_or(0);
            let start = std::time::Instant::now();
            let first = client.call(&request_for("discover.watch", Value::Null, g)).await;
            let Some(mut after) = first.data.as_ref().and_then(|d| d["after_seq"].as_i64()) else {
                return Ok(Some(first));
            };
            loop {
                let env = client.call(&request_for("events.poll", json!({"after_seq": after, "limit": 200}), g)).await;
                if let Some(e) = env.error {
                    return Err(e);
                }
                for ev in env.data.as_ref().and_then(|d| d["events"].as_array()).cloned().unwrap_or_default() {
                    after = after.max(ev["seq"].as_i64().unwrap_or(after));
                    if g.output.is_machine() {
                        print_ndjson_line(&ev);
                    } else {
                        println!("{} {} {}", ev["at"].as_str().unwrap_or(""), ev["topic"].as_str().unwrap_or(""), ev["payload"]);
                    }
                }
                if secs > 0 && start.elapsed().as_secs() as i64 >= secs {
                    return Ok(None);
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
        "jobs_watch" | "jobs_wait" => {
            let job_id = leaf.get_one::<String>("id").cloned().unwrap_or_default();
            let mut after = 0i64;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(g.timeout_secs.max(5) * 10);
            loop {
                let env = client
                    .call(&request_for("jobs.watch", json!({"job_id": job_id, "after_sequence": after}), g))
                    .await;
                let Some(data) = env.data.clone() else { return Ok(Some(env)) };
                for ev in data["events"].as_array().cloned().unwrap_or_default() {
                    after = after.max(ev["sequence"].as_i64().unwrap_or(after));
                    if name == "jobs_watch" {
                        let mut line = json!({"event": ev["event"], "job_id": job_id, "sequence": ev["sequence"], "at": ev["at"]});
                        if let (Some(l), Some(p)) = (line.as_object_mut(), ev["payload_redacted"].as_object()) {
                            for (k, v) in p {
                                l.insert(k.clone(), v.clone());
                            }
                        }
                        if g.output.is_machine() {
                            print_ndjson_line(&line);
                        } else {
                            eprintln!("{} {}", ev["event"].as_str().unwrap_or(""), ev["payload_redacted"]);
                        }
                    }
                }
                if data["terminal"] == true {
                    let final_env = client.call(&request_for("jobs.show", json!({"job_id": job_id}), g)).await;
                    if name == "jobs_watch" && g.output == Output::Ndjson {
                        return Ok(None);
                    }
                    let mut final_env = final_env;
                    if final_env.ok && final_env.data.as_ref().is_some_and(|d| d["state"] == "failed") {
                        let d = final_env.data.clone().unwrap_or(Value::Null);
                        let mut e = OcError::new(openclaim_domain::ErrorCode::Unavailable, d["error_message"].as_str().unwrap_or("job failed"));
                        e.details = Some(d);
                        final_env.error = Some(e);
                        final_env.ok = false;
                    }
                    return Ok(Some(final_env));
                }
                if std::time::Instant::now() > deadline {
                    return Err(
                        OcError::new(openclaim_domain::ErrorCode::Timeout, "stopped waiting; the job continues in the daemon")
                            .with_next_action(format!("openclaim jobs show {job_id}")),
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
        }
        "schema_export" => {
            let env = client.call(&request_for("schema.export", Value::Null, g)).await;
            if let (Some(file), Some(data)) = (leaf.get_one::<String>("file"), env.data.as_ref()) {
                std::fs::write(file, serde_json::to_string_pretty(data).unwrap_or_default()).map_err(|e| OcError::invalid(format!("write {file}: {e}")))?;
                return Ok(Some(envelope_from_local(
                    json!({"written": file, "operations": data["operations"].as_array().map(|a| a.len()).unwrap_or(0)}),
                    g,
                )));
            }
            Ok(Some(env))
        }
        "mcp_serve" => {
            let transport = leaf.get_one::<String>("transport").map(String::as_str).unwrap_or("stdio");
            if transport != "stdio" {
                return Err(OcError::invalid("only the stdio transport is supported; no network MCP listener exists"));
            }
            client.ensure().await?;
            let c = Client::new(g);
            openclaim_mcp_adapter::serve_stdio(c, g.agent_token.clone()).await.map_err(OcError::internal)?;
            Ok(None)
        }
        "mcp_inspect" => Ok(Some(envelope_from_local(openclaim_mcp_adapter::capabilities(), g))),
        "mcp_list_tools" => Ok(Some(envelope_from_local(
            json!({"items": openclaim_mcp_adapter::tool_list().into_iter().map(|t| json!({"name": t["name"], "description": t["description"], "operation": t["_openclaim"]["operation"], "scope": t["_openclaim"]["scope"]})).collect::<Vec<_>>()}),
            g,
        ))),
        "mcp_test" => Ok(Some(envelope_from_local(openclaim_mcp_adapter::self_test(), g))),
        "completion" => {
            let shell = match spec.name {
                "powershell" => clap_complete::Shell::PowerShell,
                "bash" => clap_complete::Shell::Bash,
                _ => clap_complete::Shell::Zsh,
            };
            let mut cmd = build_command();
            clap_complete::generate(shell, &mut cmd, "openclaim", &mut std::io::stdout());
            Ok(None)
        }
        "gui_open" => {
            client.ensure().await?;
            let info = client.daemon_info().ok_or_else(|| OcError::unavailable("daemon info file missing"))?;
            let port = info
                .http_port
                .ok_or_else(|| OcError::unavailable("the daemon is running without the HTTP API"))?;
            let url = format!("http://127.0.0.1:{port}/");
            if !leaf.get_flag("no-browser") {
                open_browser(&url);
            }
            Ok(Some(envelope_from_local(json!({"url": url, "note": "sign in with the owner passphrase"}), g)))
        }
        other => Err(OcError::internal(format!("no local handler for {other}"))),
    }
}

async fn openclaim_ipc_ping(client: &Client) -> bool {
    openclaim_ipc::ping(&client.endpoint).await
}

async fn wait_for_daemon(client: &Client) -> bool {
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if openclaim_ipc_ping(client).await {
            return true;
        }
    }
    false
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}
