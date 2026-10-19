//! Output contracts (MDR §16.1): `json` = exactly one envelope on stdout;
//! `ndjson` = one JSON object per line; human modes put diagnostics on stderr.

use crate::Output;
use openclaim_domain::Envelope;
use serde_json::Value;
use std::io::Write;

pub fn print_envelope(env: &Envelope, out: Output) -> i32 {
    let code = env.exit_code();
    match out {
        Output::Json => {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{}", serde_json::to_string_pretty(env).unwrap_or_default());
        }
        Output::Ndjson => {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{}", serde_json::to_string(env).unwrap_or_default());
        }
        Output::Table | Output::Text => {
            for w in &env.warnings {
                eprintln!("warning: {w}");
            }
            if let Some(e) = &env.error {
                eprintln!("error {} ({}): {}", e.code.as_str(), code, e.message);
                if let Some(a) = &e.approval_id {
                    eprintln!("approval id: {a}");
                }
                if !e.needed_scopes.is_empty() {
                    eprintln!("needed scopes: {}", e.needed_scopes.join(", "));
                }
                if let Some(n) = &e.next_action {
                    eprintln!("next: {n}");
                }
                if let Some(d) = &e.details {
                    eprintln!("details: {}", serde_json::to_string(d).unwrap_or_default());
                }
            } else if let Some(d) = &env.data {
                let mut stdout = std::io::stdout().lock();
                let _ = write!(stdout, "{}", render(d, out == Output::Table));
            }
            if let Some(p) = &env.pagination {
                if let Some(c) = &p.next_cursor {
                    eprintln!("more results: --cursor {c}");
                }
            }
            if env.dry_run {
                eprintln!("(dry run)");
            }
        }
    }
    code
}

fn cell(v: &Value) -> String {
    let s = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "yes".into() } else { "no".into() },
        other => other.to_string(),
    };
    let s = s.replace('\n', " ");
    if s.chars().count() > 48 { format!("{}…", s.chars().take(47).collect::<String>()) } else { s }
}

/// Renders `{items: [...]}` as a column table, other objects as `key: value` lines.
pub fn render(v: &Value, table: bool) -> String {
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        let mut out = render_items(items, table);
        for (k, val) in v.as_object().unwrap() {
            if k != "items" && k != "pagination" {
                out.push_str(&format!("{k}: {}\n", cell(val)));
            }
        }
        return out;
    }
    if let Some(content) = v.get("content").and_then(|c| c.as_str()) {
        return format!("{content}\n");
    }
    if let Some(ics) = v.get("ics").and_then(|c| c.as_str()) {
        return format!("{ics}\n");
    }
    match v {
        Value::Object(m) => {
            let width = m.keys().map(|k| k.len()).max().unwrap_or(0).min(28);
            let mut out = String::new();
            for (k, val) in m {
                match val {
                    Value::Array(a) if a.iter().all(|x| x.is_object()) && !a.is_empty() && table => {
                        out.push_str(&format!("{k}:\n{}", indent(&render_items(a, true))));
                    }
                    Value::Object(_) => out.push_str(&format!("{k}:\n{}", indent(&render(val, table)))),
                    _ => out.push_str(&format!("{:width$}  {}\n", k, cell(val), width = width)),
                }
            }
            out
        }
        Value::Array(a) => a.iter().map(|x| format!("{}\n", cell(x))).collect(),
        other => format!("{}\n", cell(other)),
    }
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("  {l}\n")).collect()
}

fn render_items(items: &[Value], table: bool) -> String {
    if items.is_empty() {
        return "(no results)\n".into();
    }
    if !table {
        return items.iter().map(|i| format!("{}\n", serde_json::to_string(i).unwrap_or_default())).collect();
    }
    const PREFERRED: &[&str] = &["id", "title", "name", "display_name", "status", "state", "verification_state", "assessment", "kind", "jurisdiction", "claim_deadline", "due_at", "health", "op", "decision", "updated_at", "created_at"];
    let first = items[0].as_object().cloned().unwrap_or_default();
    let mut cols: Vec<String> = PREFERRED.iter().filter(|c| first.contains_key(**c)).map(|c| c.to_string()).collect();
    for (k, v) in &first {
        if cols.len() >= 7 {
            break;
        }
        if !cols.contains(k) && !v.is_object() && !v.is_array() {
            cols.push(k.clone());
        }
    }
    let rows: Vec<Vec<String>> = items.iter().map(|it| cols.iter().map(|c| cell(&it[c])).collect()).collect();
    let widths: Vec<usize> = cols.iter().enumerate().map(|(i, c)| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0).max(c.len())).collect();
    let mut out = String::new();
    out.push_str(&cols.iter().enumerate().map(|(i, c)| format!("{:width$}", c, width = widths[i])).collect::<Vec<_>>().join("  "));
    out.push('\n');
    out.push_str(&widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  "));
    out.push('\n');
    for r in rows {
        out.push_str(&r.iter().enumerate().map(|(i, c)| format!("{:width$}", c, width = widths[i])).collect::<Vec<_>>().join("  "));
        out.push('\n');
    }
    out
}

pub fn print_ndjson_line(v: &Value) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", serde_json::to_string(v).unwrap_or_default());
}
