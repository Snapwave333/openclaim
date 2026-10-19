//! Golden CLI contract tests (MDR §14.8): machine output shape and deterministic exit
//! codes, exercised on the real binary without a daemon.

use serde_json::Value;
use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_openclaim"))
        .args(args)
        .env("OPENCLAIM_HOME", dir.path())
        .env("OPENCLAIM_NO_AUTOSTART", "1")
        .env_remove("OPENCLAIM_SESSION")
        .env_remove("OPENCLAIM_AGENT_TOKEN")
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn json_mode_prints_exactly_one_envelope_even_on_failure() {
    let (code, stdout, stderr) = run(&["schema", "version", "--output", "json"]);
    assert_eq!(code, 7, "no daemon and no autostart → UNAVAILABLE; stderr: {stderr}");
    let env: Value = serde_json::from_str(stdout.trim()).expect("stdout is one JSON object");
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "UNAVAILABLE");
    assert_eq!(env["schema_version"], "1.1");
    assert!(
        stderr.trim().is_empty() || !stderr.contains('{'),
        "diagnostics never leak JSON onto stderr as prose"
    );
}

#[test]
fn ndjson_mode_is_one_object_per_line() {
    let (_, stdout, _) = run(&["help-tree", "--output", "ndjson"]);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let v: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["ok"], true);
}

#[test]
fn help_tree_lists_the_whole_command_surface() {
    let (code, stdout, _) = run(&["help-tree", "--output", "json"]);
    assert_eq!(code, 0);
    let env: Value = serde_json::from_str(&stdout).unwrap();
    let cmds = env["data"]["commands"].as_array().unwrap();
    assert_eq!(cmds.len(), openclaim_cli_core::spec::COMMANDS.len());
    let exit_codes = env["data"]["exit_codes"].as_array().unwrap();
    assert_eq!(exit_codes.iter().find(|e| e["code"] == "APPROVAL_REQUIRED").unwrap()["exit_code"], 5);
    assert!(env["data"]["global_flags"].as_array().unwrap().iter().any(|f| f == "--dry-run"));
}

#[test]
fn usage_errors_exit_2_and_help_exits_0() {
    let (code, _, stderr) = run(&["opportunities", "show"]);
    assert_eq!(code, 2, "{stderr}");
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("discover"));
    let (code, _, _) = run(&["no-such-command"]);
    assert_eq!(code, 2);
}

#[test]
fn daemon_status_reports_unreachable_without_starting_anything() {
    let (code, stdout, _) = run(&["daemon", "status", "--output", "json"]);
    assert_eq!(code, 0);
    let env: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(env["data"]["reachable"], false);
}

#[test]
fn mcp_self_test_and_tool_list_need_no_daemon() {
    let (code, stdout, _) = run(&["mcp", "test", "--output", "json"]);
    assert_eq!(code, 0);
    let env: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(env["data"]["ok"], true);
    let (code, stdout, _) = run(&["mcp", "list-tools", "--output", "json"]);
    assert_eq!(code, 0);
    let env: Value = serde_json::from_str(&stdout).unwrap();
    assert!(env["data"]["items"].as_array().unwrap().len() >= 18);
}

#[test]
fn completions_generate() {
    for shell in ["powershell", "bash", "zsh"] {
        let (code, stdout, _) = run(&["completion", shell]);
        assert_eq!(code, 0, "{shell}");
        assert!(stdout.contains("openclaim"));
    }
}
