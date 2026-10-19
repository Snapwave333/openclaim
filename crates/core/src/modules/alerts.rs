//! Deadline reminders and change alerts (MDR §5.7). Reminders are only
//! scheduled for verified deadlines; delivery is local (events + ICS export).

use super::{IdParam, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require};
use openclaim_db::rusqlite::Connection;
use openclaim_domain::dates::{format_date, parse_date};
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Creates lead-day reminders for a bookmarked/matched opportunity's verified deadline.
pub fn schedule_deadline_alerts(conn: &Connection, subject_kind: &str, opportunity_id: &str, now: &str) -> OcResult<u32> {
    let opp = require(
        query_one(
            conn,
            "SELECT id, title, claim_deadline, deadline_verified, verification_state FROM opportunities WHERE id = ?1",
            [opportunity_id],
        )?,
        "opportunity",
        opportunity_id,
    )?;
    let Some(deadline) = opp["claim_deadline"].as_str() else { return Ok(0) };
    if opp["deadline_verified"] != true || opp["verification_state"] != "verified" {
        return Ok(0);
    }
    let d = parse_date(deadline)?;
    let lead: Vec<i64> = crate::modules::config::get(conn, "alerts.lead_days")?
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_else(|| vec![30, 14, 7, 3, 1]);
    let today = openclaim_domain::dates::today_utc();
    let mut n = 0;
    for days in lead {
        let Some(due) = d.checked_sub(time::Duration::days(days)) else { continue };
        if due < today {
            continue;
        }
        let key = format!("deadline:{opportunity_id}:{days}");
        let title = format!("{} — claim deadline {} in {} day(s)", opp["title"].as_str().unwrap_or(""), deadline, days);
        n += exec(
            conn,
            "INSERT OR IGNORE INTO alerts(id, subject_kind, subject_id, kind, title, due_at, dedupe_key, created_at) VALUES (?1,?2,?3,'deadline',?4,?5,?6,?7)",
            (
                openclaim_db::new_id(),
                subject_kind,
                opportunity_id,
                title,
                format!("{}T09:00:00Z", format_date(due)),
                key,
                now,
            ),
        )? as u32;
    }
    Ok(n)
}

pub fn change_alert(conn: &Connection, opportunity_id: &str, title: &str, detail: &str, now: &str) -> OcResult<()> {
    let tracked: i64 = crate::sql::scalar::<i64, _>(
        conn,
        "SELECT (SELECT count(*) FROM claims WHERE opportunity_id = ?1) + (SELECT count(*) FROM opportunity_marks WHERE opportunity_id = ?1 AND bookmarked = 1) + (SELECT count(*) FROM matches WHERE opportunity_id = ?1 AND assessment = 'potential_match')",
        [opportunity_id],
    )?
    .unwrap_or(0);
    if tracked == 0 {
        return Ok(());
    }
    exec(
        conn,
        "INSERT INTO alerts(id, subject_kind, subject_id, kind, title, due_at, created_at) VALUES (?1,'opportunity',?2,'change',?3,?4,?4)",
        (openclaim_db::new_id(), opportunity_id, format!("{title}: {detail}"), now),
    )?;
    // Stale deadline reminders are dropped; new ones are scheduled from the verified deadline.
    exec(
        conn,
        "DELETE FROM alerts WHERE subject_id = ?1 AND kind = 'deadline' AND disposition = 'pending'",
        [opportunity_id],
    )?;
    schedule_deadline_alerts(conn, "opportunity", opportunity_id, now)?;
    Ok(())
}

/// Marks due reminders delivered and emits events. Run by the scheduler.
pub fn sweep(conn: &Connection, now: &str) -> OcResult<u32> {
    let due = query_all(
        conn,
        "SELECT id, title, kind, subject_kind, subject_id FROM alerts WHERE disposition = 'pending' AND due_at <= ?1 AND (snoozed_until IS NULL OR snoozed_until <= ?1)",
        [now],
    )?;
    for a in &due {
        exec(
            conn,
            "UPDATE alerts SET disposition = 'delivered', delivered_at = ?2 WHERE id = ?1",
            (a["id"].as_str().unwrap_or(""), now),
        )?;
        crate::events::emit(
            conn,
            "alert.due",
            json!({"alert_id": a["id"], "title": a["title"], "kind": a["kind"], "subject_kind": a["subject_kind"], "subject_id": a["subject_id"]}),
        )?;
    }
    for claim in query_all(
        conn,
        "SELECT c.id, c.opportunity_id FROM claims c JOIN opportunities o ON o.id = c.opportunity_id WHERE c.status NOT IN ('submitted','pending','approved','denied','paid','withdrawn','expired') AND o.deadline_verified = 1",
        [],
    )? {
        schedule_deadline_alerts(conn, "claim", claim["opportunity_id"].as_str().unwrap_or(""), now)?;
    }
    Ok(due.len() as u32)
}

#[derive(Deserialize, JsonSchema)]
pub struct ListParams {
    #[serde(default)]
    pub due_within_days: Option<u32>,
    #[serde(default)]
    pub include_dismissed: bool,
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ListParams = parse_params(p)?;
    let horizon = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() + time::Duration::days(p.due_within_days.unwrap_or(365) as i64));
    let rows = query_all(
        ctx.conn,
        "SELECT a.*, o.title AS opportunity_title, o.claim_deadline FROM alerts a LEFT JOIN opportunities o ON o.id = a.subject_id WHERE a.due_at <= ?1 AND (?2 = 1 OR a.disposition != 'dismissed') ORDER BY a.due_at",
        (&horizon, p.include_dismissed as i32),
    )?;
    Ok(json!({"items": rows, "note": "reminders exist only for verified deadlines"}))
}

fn op_due(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let rows = query_all(
        ctx.conn,
        "SELECT a.*, o.title AS opportunity_title FROM alerts a LEFT JOIN opportunities o ON o.id = a.subject_id WHERE a.disposition IN ('pending','delivered') AND a.due_at <= ?1 ORDER BY a.due_at",
        [&ctx.now],
    )?;
    Ok(json!({"items": rows}))
}

#[derive(Deserialize, JsonSchema)]
pub struct AddParams {
    /// opportunity | claim | property | custom
    pub subject_kind: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    pub title: String,
    /// RFC 3339 timestamp or YYYY-MM-DD.
    pub due_at: String,
}

fn op_add(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: AddParams = parse_params(p)?;
    let due = if p.due_at.len() == 10 {
        format!("{}T09:00:00Z", parse_date(&p.due_at).map(format_date)?)
    } else {
        openclaim_domain::dates::parse_ts(&p.due_at).map(openclaim_domain::dates::format_ts)?
    };
    let id = openclaim_db::new_id();
    exec(
        ctx.conn,
        "INSERT INTO alerts(id, subject_kind, subject_id, kind, title, due_at, created_at) VALUES (?1,?2,?3,'custom',?4,?5,?6)",
        (
            &id,
            &p.subject_kind,
            p.subject_id.unwrap_or_default(),
            openclaim_security::sanitize::clean_text(&p.title, 200),
            &due,
            &ctx.now,
        ),
    )?;
    Ok(json!({"id": id, "due_at": due}))
}

#[derive(Deserialize, JsonSchema)]
pub struct SnoozeParams {
    pub id: String,
    /// RFC 3339 timestamp or YYYY-MM-DD.
    pub until: String,
}

fn op_snooze(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SnoozeParams = parse_params(p)?;
    let until = if p.until.len() == 10 {
        format!("{}T09:00:00Z", parse_date(&p.until).map(format_date)?)
    } else {
        openclaim_domain::dates::parse_ts(&p.until).map(openclaim_domain::dates::format_ts)?
    };
    let n = exec(
        ctx.conn,
        "UPDATE alerts SET disposition = 'snoozed', snoozed_until = ?2, due_at = ?2 WHERE id = ?1",
        (&p.id, &until),
    )?;
    if n == 0 {
        return Err(OcError::not_found("alert not found"));
    }
    exec(ctx.conn, "UPDATE alerts SET disposition = 'pending' WHERE id = ?1", [&p.id])?;
    Ok(json!({"id": p.id, "due_at": until}))
}

fn op_dismiss(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let n = exec(ctx.conn, "UPDATE alerts SET disposition = 'dismissed' WHERE id = ?1", [&p.id])?;
    if n == 0 {
        return Err(OcError::not_found("alert not found"));
    }
    Ok(json!({"id": p.id, "disposition": "dismissed"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct IcsParams {
    /// Optional path to write the .ics file to.
    #[serde(default)]
    pub path: Option<String>,
}

fn ics_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

fn op_export_ics(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IcsParams = parse_params(p)?;
    let rows = query_all(
        ctx.conn,
        "SELECT id, title, due_at, kind FROM alerts WHERE disposition != 'dismissed' ORDER BY due_at",
        [],
    )?;
    let mut ics = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//OpenClaim//Reminders//EN\r\nCALSCALE:GREGORIAN\r\n");
    for r in &rows {
        let due = r["due_at"].as_str().unwrap_or("");
        let stamp: String = due.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        ics.push_str(&format!(
            "BEGIN:VEVENT\r\nUID:{}@openclaim\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nSUMMARY:{}\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\n",
            r["id"].as_str().unwrap_or(""),
            stamp,
            stamp,
            ics_escape(r["title"].as_str().unwrap_or("")),
            ics_escape("OpenClaim reminder. Verify on the official site before acting.")
        ));
    }
    ics.push_str("END:VCALENDAR\r\n");
    if let Some(path) = &p.path
        && !ctx.dry_run
    {
        std::fs::write(path, &ics).map_err(|e| OcError::invalid(format!("write {path}: {e}")))?;
    }
    Ok(json!({"count": rows.len(), "ics": ics, "path": p.path}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!(
            "alerts.list",
            "alerts.read",
            Read,
            "Upcoming reminders and change alerts",
            ListParams,
            op_list,
            mcp = "list_alerts"
        ),
        op!("alerts.due", "alerts.read", Read, "Reminders due now", super::Empty, op_due),
        op!("alerts.add", "alerts.write", Write, "Add a custom reminder", AddParams, op_add),
        op!("alerts.snooze", "alerts.write", Write, "Snooze a reminder", SnoozeParams, op_snooze),
        op!("alerts.dismiss", "alerts.write", Write, "Dismiss a reminder", IdParam, op_dismiss),
        op!(
            "alerts.export_ics",
            "alerts.read",
            Read,
            "Export reminders as an iCalendar file",
            IcsParams,
            op_export_ics
        ),
    ]
}
