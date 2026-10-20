//! Structural parity (MDR §14 release gate): every CLI row maps to a real operation,
//! every operation is reachable from the CLI, and the clap tree builds without
//! debug assertions failing.

use openclaim_cli_core::spec::{COMMANDS, Kind};
use openclaim_core::ops;
use std::collections::HashSet;

#[test]
fn every_cli_command_maps_to_a_registered_operation() {
    for c in COMMANDS {
        if c.op.starts_with("local:") {
            continue;
        }
        let op = ops::find(c.op).unwrap_or_else(|| panic!("{} {} → unknown op {}", c.group, c.name, c.op));
        let schema = (op.params_schema)();
        let props: HashSet<String> = schema["properties"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        let additional = schema["additionalProperties"].as_bool().unwrap_or(true);
        for a in c.args {
            if !additional && !props.contains(a.param) && !matches!(a.param, "limit" | "cursor") {
                panic!("{} {}: --{} sets '{}' which op {} does not accept", c.group, c.name, a.name, a.param, c.op);
            }
        }
    }
}

#[test]
fn every_operation_is_reachable_from_the_cli() {
    let covered: HashSet<&str> = COMMANDS.iter().map(|c| c.op).collect();
    let missing: Vec<&str> = ops::registry()
        .iter()
        .map(|o| o.name)
        .filter(|n| !covered.contains(n) && !local_only(n))
        .collect();
    assert!(missing.is_empty(), "operations without a CLI command: {missing:?}");
}

/// Operations the CLI reaches through a local handler rather than a direct row.
fn local_only(op: &str) -> bool {
    matches!(
        op,
        "owner.init"
            | "owner.unlock"
            | "owner.set_passphrase"
            | "daemon.stop"
            | "daemon.status"
            | "schema.export"
            | "jobs.watch"
            | "discover.watch"
            | "sources.capture_from_extension"
    )
}

#[test]
fn clap_tree_builds_and_help_json_lists_every_row() {
    let cmd = openclaim_cli_core::build_command();
    cmd.clone().debug_assert();
    let help = openclaim_cli_core::help_json();
    assert_eq!(help["commands"].as_array().unwrap().len(), COMMANDS.len());
    assert!(COMMANDS.iter().all(|c| c.args.iter().filter(|a| a.kind == Kind::Multi).count() <= 1));
    for required in [
        ["daemon", "ensure"],
        ["discover", "search"],
        ["opportunities", "show"],
        ["sources", "refresh"],
        ["match", "evaluate"],
        ["people", "create"],
        ["facts", "propose"],
        ["documents", "import"],
        ["claims", "prepare"],
        ["claims", "handoff"],
        ["claims", "record-submission"],
        ["opportunities", "verify-official"],
        ["alerts", "export-ics"],
        ["jobs", "watch"],
        ["workflows", "plan"],
        ["agents", "register"],
        ["approvals", "approve"],
        ["audit", "verify"],
        ["data", "wipe"],
        ["config", "set"],
        ["schema", "export"],
        ["mcp", "serve"],
        ["completion", "powershell"],
    ] {
        assert!(COMMANDS.iter().any(|c| c.group == required[0] && c.name == required[1]), "missing {required:?}");
    }
}

#[test]
fn json_output_is_exactly_one_envelope() {
    let g = openclaim_cli_core::globals(&openclaim_cli_core::build_command().get_matches_from(["openclaim", "--output", "json", "schema", "version"]));
    let env = openclaim_cli_core::envelope_from_local(serde_json::json!({"a": 1}), &g);
    let text = serde_json::to_string_pretty(&env).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["schema_version"], "1.1");
    assert_eq!(parsed["ok"], true);
    assert_eq!(g.output, openclaim_cli_core::Output::Json);
}
