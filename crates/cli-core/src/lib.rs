//! `openclaim` CLI: clap command tree built from `spec::COMMANDS`, strict
//! machine output modes, and the daemon client with explicit auto-start.

pub mod client;
pub mod output;
pub mod spec;

use clap::{Arg, ArgAction, ArgMatches, Command};
use openclaim_domain::{Envelope, OcError};
use serde_json::{Map, Value, json};
use spec::{ArgSpec, CmdSpec, Kind};

pub const ENV_SESSION: &str = "OPENCLAIM_SESSION";
pub const ENV_AGENT_TOKEN: &str = "OPENCLAIM_AGENT_TOKEN";
pub const ENV_HOME: &str = "OPENCLAIM_HOME";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Table,
    Text,
    Json,
    Ndjson,
}

impl Output {
    pub fn parse(s: &str) -> Option<Output> {
        match s {
            "table" => Some(Output::Table),
            "text" => Some(Output::Text),
            "json" => Some(Output::Json),
            "ndjson" => Some(Output::Ndjson),
            _ => None,
        }
    }
    pub fn is_machine(self) -> bool {
        matches!(self, Output::Json | Output::Ndjson)
    }
}

#[derive(Debug, Clone)]
pub struct Globals {
    pub output: Output,
    pub fields: Vec<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub timeout_secs: u64,
    pub offline: bool,
    pub dry_run: bool,
    pub request_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub non_interactive: bool,
    pub approval: Option<String>,
    pub data_dir: Option<std::path::PathBuf>,
    pub session: Option<String>,
    pub agent_token: Option<String>,
    pub no_autostart: bool,
}

fn arg_from(spec: &ArgSpec, index: usize) -> Arg {
    let mut a = Arg::new(spec.name).help(spec.help);
    if spec.positional {
        a = a.index(index).required(spec.required);
        if spec.kind == Kind::Multi {
            a = a.num_args(1..).action(ArgAction::Append);
        }
    } else {
        a = a.long(spec.name).required(spec.required);
    }
    match spec.kind {
        Kind::Flag => a.action(ArgAction::SetTrue),
        Kind::Int => a.value_parser(clap::value_parser!(i64)).action(ArgAction::Set),
        Kind::Multi => a,
        _ => a.action(ArgAction::Set),
    }
}

pub fn build_command() -> Command {
    let mut root = Command::new("openclaim")
        .version(openclaim_domain::VERSION)
        .about("Free, local-first settlement discovery and claims assistant. No AI inside; agents drive it from outside.")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("output")
                .long("output")
                .short('o')
                .global(true)
                .value_parser(["table", "text", "json", "ndjson"])
                .default_value("table")
                .help("Output format"),
        )
        .arg(
            Arg::new("fields")
                .long("fields")
                .global(true)
                .help("Comma-separated fields to keep in list output"),
        )
        .arg(
            Arg::new("limit")
                .long("limit")
                .global(true)
                .value_parser(clap::value_parser!(u32))
                .help("Page size"),
        )
        .arg(Arg::new("cursor").long("cursor").global(true).help("Pagination cursor"))
        .arg(
            Arg::new("timeout")
                .long("timeout")
                .global(true)
                .value_parser(clap::value_parser!(u64))
                .default_value("60")
                .help("Seconds to wait for the daemon"),
        )
        .arg(
            Arg::new("offline")
                .long("offline")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Refuse any network side effect"),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Preview without committing"),
        )
        .arg(Arg::new("request-id").long("request-id").global(true).help("Caller-chosen request id"))
        .arg(
            Arg::new("idempotency-key")
                .long("idempotency-key")
                .global(true)
                .help("Safe retries for mutations"),
        )
        .arg(
            Arg::new("non-interactive")
                .long("non-interactive")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Never prompt; return APPROVAL_REQUIRED instead"),
        )
        .arg(
            Arg::new("approval")
                .long("approval")
                .global(true)
                .help("Approval id to consume for this action"),
        )
        .arg(Arg::new("profile").long("profile").global(true).help("Named profile (separate data directory)"))
        .arg(
            Arg::new("data-dir")
                .long("data-dir")
                .global(true)
                .help("Data directory (overrides OPENCLAIM_HOME)"),
        )
        .arg(
            Arg::new("session")
                .long("session")
                .global(true)
                .env(ENV_SESSION)
                .hide_env_values(true)
                .help("Owner session token"),
        )
        .arg(
            Arg::new("agent-token")
                .long("agent-token")
                .global(true)
                .env(ENV_AGENT_TOKEN)
                .hide_env_values(true)
                .help("Registered agent token"),
        )
        .arg(
            Arg::new("no-autostart")
                .long("no-autostart")
                .global(true)
                .action(ArgAction::SetTrue)
                .env("OPENCLAIM_NO_AUTOSTART")
                .help("Do not start the daemon automatically"),
        );
    for c in spec::COMMANDS.iter().filter(|c| c.group.is_empty()) {
        root = root.subcommand(leaf(c));
    }
    for g in spec::groups() {
        let about = spec::GROUP_ABOUT.iter().find(|(n, _)| *n == g).map(|(_, a)| *a).unwrap_or("");
        let mut grp = Command::new(g).about(about).subcommand_required(true).arg_required_else_help(true);
        for c in spec::COMMANDS.iter().filter(|c| c.group == g) {
            grp = grp.subcommand(leaf(c));
        }
        root = root.subcommand(grp);
    }
    root.subcommand(Command::new("help-tree").hide(true).about("Command tree as JSON"))
}

fn leaf(c: &CmdSpec) -> Command {
    let mut cmd = Command::new(c.name).about(c.about);
    let mut index = 0;
    for a in c.args {
        if a.positional {
            index += 1;
        }
        cmd = cmd.arg(arg_from(a, index));
    }
    cmd
}

pub fn globals(m: &ArgMatches) -> Globals {
    let data_dir = m.get_one::<String>("data-dir").map(std::path::PathBuf::from).or_else(|| {
        m.get_one::<String>("profile").map(|p| {
            let base = openclaim_core::Paths::resolve(None).data_dir;
            base.join("profiles").join(p)
        })
    });
    Globals {
        output: Output::parse(m.get_one::<String>("output").map(String::as_str).unwrap_or("table")).unwrap_or(Output::Table),
        fields: m
            .get_one::<String>("fields")
            .map(|f| f.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default(),
        limit: m.get_one::<u32>("limit").copied(),
        cursor: m.get_one::<String>("cursor").cloned(),
        timeout_secs: m.get_one::<u64>("timeout").copied().unwrap_or(60),
        offline: m.get_flag("offline"),
        dry_run: m.get_flag("dry-run"),
        request_id: m.get_one::<String>("request-id").cloned(),
        idempotency_key: m.get_one::<String>("idempotency-key").cloned(),
        non_interactive: m.get_flag("non-interactive") || !std::io::IsTerminal::is_terminal(&std::io::stdin()),
        approval: m.get_one::<String>("approval").cloned(),
        data_dir,
        session: m.get_one::<String>("session").cloned().filter(|s| !s.is_empty()),
        agent_token: m.get_one::<String>("agent-token").cloned().filter(|s| !s.is_empty()),
        no_autostart: m.get_flag("no-autostart"),
    }
}

/// Finds the leaf command and its matches.
pub fn resolve(m: &ArgMatches) -> Option<(&'static CmdSpec, &ArgMatches)> {
    let (first, sub) = m.subcommand()?;
    if let Some(c) = spec::COMMANDS.iter().find(|c| c.group.is_empty() && c.name == first) {
        return Some((c, sub));
    }
    let (second, leaf) = sub.subcommand()?;
    spec::COMMANDS.iter().find(|c| c.group == first && c.name == second).map(|c| (c, leaf))
}

pub fn read_json_arg(raw: &str) -> Result<Value, OcError> {
    let text = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path).map_err(|e| OcError::invalid(format!("read {path}: {e}")))?
    } else {
        raw.to_string()
    };
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        return Ok(v);
    }
    serde_yaml::from_str::<Value>(&text).map_err(|e| OcError::invalid(format!("not JSON or YAML: {e}")))
}

/// Builds operation params from a leaf's matches.
pub fn params_from(c: &CmdSpec, m: &ArgMatches, g: &Globals) -> Result<Value, OcError> {
    let mut p = Map::new();
    for a in c.args {
        let v: Option<Value> = match a.kind {
            Kind::Flag => {
                let on = m.get_flag(a.name);
                // `--off`/`--negative`/`--clear` flags invert the parameter they set.
                if matches!(a.name, "off" | "negative" | "clear") {
                    Some(json!(!on))
                } else if on {
                    Some(json!(true))
                } else {
                    None
                }
            }
            Kind::Int => m.get_one::<i64>(a.name).map(|n| json!(n)),
            Kind::BoolVal => m.get_one::<String>(a.name).map(|s| json!(matches!(s.as_str(), "true" | "yes" | "1"))),
            Kind::Json => match m.get_one::<String>(a.name) {
                Some(raw) => Some(read_json_arg(raw)?),
                None => None,
            },
            Kind::FileText => match m.get_one::<String>(a.name) {
                Some(path) => Some(json!(std::fs::read_to_string(path).map_err(|e| OcError::invalid(format!("read {path}: {e}")))?)),
                None => None,
            },
            Kind::List => m
                .get_one::<String>(a.name)
                .map(|s| json!(s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>())),
            Kind::Multi => m.get_many::<String>(a.name).map(|vals| json!(vals.cloned().collect::<Vec<_>>())),
            Kind::Str => m.get_one::<String>(a.name).map(|s| json!(s)),
        };
        if let Some(v) = v {
            p.insert(a.param.to_string(), v);
        }
    }
    if let Some(l) = g.limit {
        p.insert("limit".into(), json!(l));
    }
    if let Some(c) = &g.cursor {
        p.insert("cursor".into(), json!(c));
    }
    if !g.fields.is_empty() && c.op.contains("list") {
        p.insert("fields".into(), json!(g.fields));
    }
    Ok(Value::Object(p))
}

pub fn request_for(op: &str, params: Value, g: &Globals) -> openclaim_core::Request {
    let mut req = openclaim_core::Request::new(op, params);
    req.credentials.session = g.session.clone();
    req.credentials.agent_token = g.agent_token.clone();
    req.request_id = g.request_id.clone();
    req.dry_run = g.dry_run;
    req.non_interactive = g.non_interactive;
    req.offline = g.offline;
    req.idempotency_key = g.idempotency_key.clone();
    req.approval_id = g.approval.clone();
    req.transport = "cli".into();
    req
}

/// `help --output json` / `help-tree`: the whole tree with argument metadata.
pub fn help_json() -> Value {
    let cmds: Vec<Value> = spec::COMMANDS
        .iter()
        .map(|c| {
            json!({
                "path": if c.group.is_empty() { vec![c.name] } else { vec![c.group, c.name] },
                "operation": c.op,
                "about": c.about,
                "args": c.args.iter().map(|a| json!({"name": a.name, "param": a.param, "kind": format!("{:?}", a.kind).to_lowercase(), "positional": a.positional, "required": a.required, "help": a.help})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "product": "openclaim",
        "version": openclaim_domain::VERSION,
        "schema_version": openclaim_domain::SCHEMA_VERSION,
        "global_flags": ["--output", "--fields", "--limit", "--cursor", "--timeout", "--offline", "--dry-run", "--request-id", "--idempotency-key", "--non-interactive", "--approval", "--profile", "--data-dir", "--session", "--agent-token", "--no-autostart"],
        "exit_codes": openclaim_core::modules::schema::error_codes(),
        "commands": cmds,
    })
}

pub fn envelope_from_local(data: Value, g: &Globals) -> Envelope {
    let mut e = Envelope::success(
        g.request_id.clone().unwrap_or_else(openclaim_core::new_request_id),
        openclaim_core::new_request_id(),
        data,
    );
    e.dry_run = g.dry_run;
    e
}
