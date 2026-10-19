//! Catalog search, detail, provenance, curated rules, marks and tips.

use super::{IdParam, PageParams, Pager, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, fts_query, like_pattern, query_all, query_one, require, scalar};
use openclaim_db::rusqlite::types::Value as SqlValue;
use openclaim_domain::model::{OpportunityStatus, VerificationState};
use openclaim_domain::rules::RuleSet;
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Typed filter vocabulary (MDR §17). `filter` accepts the same keys as `key=value key=value`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct OpportunityQuery {
    /// Full-text query over title, administrator, summary and case number.
    #[serde(default)]
    pub q: Option<String>,
    /// One or more of open, upcoming, automatic_distribution, past_deadline, closed, awaiting_administrator, unknown.
    #[serde(default)]
    pub status: Vec<String>,
    /// Jurisdiction code (e.g. CA, US).
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub only_verified: bool,
    #[serde(default)]
    pub deadline_after: Option<String>,
    #[serde(default)]
    pub deadline_before: Option<String>,
    #[serde(default)]
    pub verified_since: Option<String>,
    #[serde(default)]
    pub benefit_type: Option<String>,
    /// online_form, automatic, mail, unknown
    #[serde(default)]
    pub claim_method: Option<String>,
    /// none, optional, required, unknown
    #[serde(default)]
    pub proof_required: Option<String>,
    #[serde(default)]
    pub administrator: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    /// healthy, degraded, blocked, …
    #[serde(default)]
    pub source_health: Option<String>,
    /// Requires person_id: potential_match, needs_more_information, unlikely, not_applicable, manual_review.
    #[serde(default)]
    pub match_state: Option<String>,
    #[serde(default)]
    pub person_id: Option<String>,
    #[serde(default)]
    pub bookmarked: Option<bool>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub include_archived: bool,
    /// deadline | updated | verified | title | activity
    #[serde(default)]
    pub sort: Option<String>,
    /// Compact filter string: `status=open only_verified deadline_before=2027-01-01`.
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(flatten)]
    pub page: PageParams,
}

impl OpportunityQuery {
    pub fn apply_filter_string(&mut self) -> OcResult<()> {
        let Some(f) = self.filter.take() else { return Ok(()) };
        for token in f.split_whitespace() {
            let (k, v) = match token.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (token, None),
            };
            match (k, v) {
                ("only_verified", None) | ("only_verified", Some("true")) => self.only_verified = true,
                ("include_hidden", _) => self.include_hidden = true,
                ("bookmarked", _) => self.bookmarked = Some(v != Some("false")),
                ("status", Some(v)) => self.status.extend(v.split(',').map(str::to_string)),
                ("state", Some(v)) => self.state = Some(v.into()),
                ("q", Some(v)) => self.q = Some(v.into()),
                ("deadline_after", Some(v)) => self.deadline_after = Some(v.into()),
                ("deadline_before", Some(v)) => self.deadline_before = Some(v.into()),
                ("verified_since", Some(v)) => self.verified_since = Some(v.into()),
                ("claim_method", Some(v)) => self.claim_method = Some(v.into()),
                ("proof_required", Some(v)) => self.proof_required = Some(v.into()),
                ("administrator", Some(v)) => self.administrator = Some(v.into()),
                ("source", Some(v)) | ("source_id", Some(v)) => self.source_id = Some(v.into()),
                ("source_health", Some(v)) => self.source_health = Some(v.into()),
                ("match_state", Some(v)) => self.match_state = Some(v.into()),
                ("person", Some(v)) | ("person_id", Some(v)) => self.person_id = Some(v.into()),
                ("sort", Some(v)) => self.sort = Some(v.into()),
                _ => return Err(OcError::invalid(format!("unknown filter token '{token}'"))),
            }
        }
        Ok(())
    }
}

pub fn search(ctx: &Ctx<'_>, mut q: OpportunityQuery) -> OcResult<Value> {
    q.apply_filter_string()?;
    let pager = Pager::from(&q.page, &ctx.now)?;
    let mut sql = String::from(
        "SELECT o.id, o.title, o.opp_type, o.administrator, o.case_number, o.official_url, o.claim_url, o.status, o.verification_state, o.authority_tier,
                o.claim_deadline, o.deadline_verified, o.class_period_start, o.class_period_end, o.jurisdictions_json, o.summary, o.benefit_json, o.proof_required,
                o.claim_method, o.source_updated_on, o.source_id, o.verified_at, o.updated_at, o.first_seen_at, o.version,
                COALESCE(m.bookmarked, 0) AS bookmarked, COALESCE(m.hidden, 0) AS hidden, s.health AS source_health",
    );
    let mut params: Vec<SqlValue> = Vec::new();
    let push = |params: &mut Vec<SqlValue>, v: SqlValue| -> String {
        params.push(v);
        format!("?{}", params.len())
    };
    let mut from = String::from(" FROM opportunities o LEFT JOIN opportunity_marks m ON m.opportunity_id = o.id LEFT JOIN sources s ON s.id = o.source_id");
    let mut wheres: Vec<String> = vec![format!("o.first_seen_at <= {}", push(&mut params, SqlValue::Text(pager.snapshot.clone())))];
    if let Some(ms) = &q.match_state {
        let pid = q.person_id.clone().ok_or_else(|| OcError::invalid("match_state needs person_id"))?;
        ctx.require_scope("match.evaluate", None)?;
        sql.push_str(", mt.assessment AS match_state, mt.stale AS match_stale");
        from.push_str(&format!(
            " JOIN matches mt ON mt.opportunity_id = o.id AND mt.person_id = {}",
            push(&mut params, SqlValue::Text(pid))
        ));
        wheres.push(format!("mt.assessment = {}", push(&mut params, SqlValue::Text(ms.clone()))));
    } else if let Some(pid) = &q.person_id {
        ctx.require_scope("match.evaluate", None)?;
        sql.push_str(", mt.assessment AS match_state, mt.stale AS match_stale");
        from.push_str(&format!(
            " LEFT JOIN matches mt ON mt.opportunity_id = o.id AND mt.person_id = {}",
            push(&mut params, SqlValue::Text(pid.clone()))
        ));
    }
    if let Some(text) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let fq = fts_query(text);
        if fq.is_empty() {
            wheres.push(format!("o.title LIKE {} ESCAPE '\\'", push(&mut params, SqlValue::Text(like_pattern(text)))));
        } else {
            wheres.push(format!(
                "o.id IN (SELECT opportunity_id FROM opportunities_fts WHERE opportunities_fts MATCH {})",
                push(&mut params, SqlValue::Text(fq))
            ));
        }
    }
    if !q.status.is_empty() {
        let mut parts = Vec::new();
        for s in &q.status {
            OpportunityStatus::parse(s).ok_or_else(|| OcError::invalid(format!("unknown status '{s}'")))?;
            parts.push(push(&mut params, SqlValue::Text(s.clone())));
        }
        wheres.push(format!("o.status IN ({})", parts.join(",")));
    }
    if q.only_verified {
        wheres.push("o.verification_state = 'verified'".into());
    }
    if let Some(st) = &q.state {
        wheres.push(format!(
            "(o.jurisdictions_json LIKE {} OR o.jurisdictions_json LIKE '%\"US\"%')",
            push(&mut params, SqlValue::Text(format!("%\"{}\"%", st.to_uppercase())))
        ));
    }
    if let Some(d) = &q.deadline_after {
        openclaim_domain::dates::parse_date(d)?;
        wheres.push(format!("o.claim_deadline >= {}", push(&mut params, SqlValue::Text(d.clone()))));
    }
    if let Some(d) = &q.deadline_before {
        openclaim_domain::dates::parse_date(d)?;
        wheres.push(format!("o.claim_deadline <= {}", push(&mut params, SqlValue::Text(d.clone()))));
    }
    if let Some(d) = &q.verified_since {
        wheres.push(format!("o.verified_at >= {}", push(&mut params, SqlValue::Text(d.clone()))));
    }
    if let Some(v) = &q.claim_method {
        wheres.push(format!("o.claim_method = {}", push(&mut params, SqlValue::Text(v.clone()))));
    }
    if let Some(v) = &q.proof_required {
        wheres.push(format!("o.proof_required = {}", push(&mut params, SqlValue::Text(v.clone()))));
    }
    if let Some(v) = &q.benefit_type {
        wheres.push(format!("o.opp_type = {}", push(&mut params, SqlValue::Text(v.clone()))));
    }
    if let Some(v) = &q.administrator {
        wheres.push(format!(
            "o.administrator LIKE {} ESCAPE '\\'",
            push(&mut params, SqlValue::Text(like_pattern(v)))
        ));
    }
    if let Some(v) = &q.source_id {
        wheres.push(format!("o.source_id = {}", push(&mut params, SqlValue::Text(v.clone()))));
    }
    if let Some(v) = &q.source_health {
        wheres.push(format!("s.health = {}", push(&mut params, SqlValue::Text(v.clone()))));
    }
    if let Some(b) = q.bookmarked {
        wheres.push(format!("COALESCE(m.bookmarked,0) = {}", if b { 1 } else { 0 }));
    }
    if !q.include_hidden {
        wheres.push("COALESCE(m.hidden,0) = 0".into());
    }
    if !q.include_archived {
        wheres.push("o.archived_at IS NULL".into());
    }
    let order = match q.sort.as_deref().unwrap_or("updated") {
        "deadline" => "CASE WHEN o.claim_deadline IS NULL THEN 1 ELSE 0 END, o.claim_deadline ASC, o.title",
        "verified" => "o.verified_at DESC, o.title",
        "title" => "o.title ASC",
        "activity" => "COALESCE(o.source_updated_on, substr(o.updated_at,1,10)) DESC, o.updated_at DESC, o.title",
        "updated" => "o.updated_at DESC, o.title",
        other => return Err(OcError::invalid(format!("unknown sort '{other}'"))),
    };
    let count_sql = format!("SELECT count(*){from} WHERE {}", wheres.join(" AND "));
    let total: i64 = {
        let mut stmt = ctx.conn.prepare(&count_sql).map_err(openclaim_db::db_err)?;
        stmt.query_row(openclaim_db::rusqlite::params_from_iter(params.iter()), |r| r.get(0))
            .map_err(openclaim_db::db_err)?
    };
    let full = format!("{sql}{from} WHERE {} ORDER BY {order}{}", wheres.join(" AND "), pager.sql_limit());
    let mut stmt = ctx.conn.prepare(&full).map_err(openclaim_db::db_err)?;
    let rows = stmt
        .query_map(openclaim_db::rusqlite::params_from_iter(params.iter()), crate::sql::row_json)
        .map_err(openclaim_db::db_err)?;
    let mut items: Vec<Value> = rows.map(|r| r.map_err(openclaim_db::db_err)).collect::<OcResult<_>>()?;
    for it in &mut items {
        decorate(it);
        if !q.fields.is_empty()
            && let Some(o) = it.as_object_mut()
        {
            o.retain(|k, _| q.fields.iter().any(|f| f == k) || k == "id");
        }
    }
    let mut out = pager.wrap(items, Some(total as u64));
    out["disclaimer"] = json!("Stale or unverified records are flagged; a deadline is only reliable when deadline_verified is true.");
    Ok(out)
}

/// Adds the trust displays every card must carry (MDR §10).
pub fn decorate(it: &mut Value) {
    let vs = it["verification_state"].as_str().unwrap_or("verification_pending").to_string();
    let tier = it["authority_tier"].as_str().unwrap_or("B").to_string();
    let deadline = it["claim_deadline"].as_str().map(str::to_string);
    if let Some(o) = it.as_object_mut() {
        o.insert(
            "trust".into(),
            json!({
                "primary_source": tier == "A",
                "verified_on": o.get("verified_at").cloned().unwrap_or(Value::Null),
                "verification_state": vs,
                "claim_deadline": deadline,
                "deadline_verified": o.get("deadline_verified").cloned().unwrap_or(json!(false)),
                "eligibility_note": "Eligibility still requires review",
            }),
        );
    }
}

fn load(ctx: &Ctx<'_>, id: &str) -> OcResult<Value> {
    let mut row = require(
        query_one(
            ctx.conn,
            "SELECT o.*, COALESCE(m.bookmarked,0) AS bookmarked, COALESCE(m.hidden,0) AS hidden, s.health AS source_health, s.name AS source_name FROM opportunities o LEFT JOIN opportunity_marks m ON m.opportunity_id = o.id LEFT JOIN sources s ON s.id = o.source_id WHERE o.id = ?1",
            [id],
        )?,
        "opportunity",
        id,
    )?;
    if let Some(o) = row.as_object_mut() {
        o.remove("dedupe_key");
    }
    decorate(&mut row);
    Ok(row)
}

#[derive(Deserialize, JsonSchema)]
pub struct ShowParams {
    pub id: String,
    /// Any of: sources, rules, provenance, changes, aliases, matches
    #[serde(default)]
    pub include: Vec<String>,
}

pub fn current_rules(conn: &openclaim_db::rusqlite::Connection, opportunity_id: &str) -> OcResult<Option<(i64, RuleSet, String)>> {
    let row = query_one(
        conn,
        "SELECT version, criteria_ast_json, original_text_excerpt FROM opportunity_rules WHERE opportunity_id = ?1 ORDER BY version DESC LIMIT 1",
        [opportunity_id],
    )?;
    match row {
        None => Ok(None),
        Some(r) => {
            let rules: RuleSet = serde_json::from_value(r["criteria_ast"].clone())?;
            Ok(Some((
                r["version"].as_i64().unwrap_or(1),
                rules,
                r["original_text_excerpt"].as_str().unwrap_or("").to_string(),
            )))
        }
    }
}

fn provenance(ctx: &Ctx<'_>, id: &str) -> OcResult<Value> {
    let evidence = query_all(
        ctx.conn,
        "SELECT e.id, e.assertion_type, e.precise_locator, e.excerpt, e.created_at, s.url AS snapshot_url, s.fetched_at, s.acquisition, s.content_hash, s.parser_version
         FROM evidence_links e JOIN source_snapshots s ON s.id = e.source_snapshot_id WHERE e.opportunity_id = ?1 ORDER BY e.created_at DESC LIMIT 200",
        [id],
    )?;
    let labeled: Vec<Value> = evidence
        .into_iter()
        .map(|mut e| {
            let text = e["excerpt"].as_str().unwrap_or("").to_string();
            let origin = e["snapshot_url"].as_str().unwrap_or("").to_string();
            e["excerpt"] = openclaim_security::sanitize::untrusted(&text, &origin, 1000);
            e
        })
        .collect();
    let changes = query_all(
        ctx.conn,
        "SELECT field, old_value, new_value, detected_at, reviewed_at FROM opportunity_changes WHERE opportunity_id = ?1 ORDER BY detected_at DESC LIMIT 100",
        [id],
    )?;
    let aliases = query_all(
        ctx.conn,
        "SELECT source_id, external_key, title FROM opportunity_aliases WHERE opportunity_id = ?1",
        [id],
    )?;
    Ok(json!({"evidence": labeled, "changes": changes, "aliases": aliases}))
}

fn op_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ShowParams = parse_params(p)?;
    let mut out = json!({"opportunity": load(ctx, &p.id)?});
    let inc = |k: &str| p.include.iter().any(|i| i == k || i == "all");
    if inc("rules") {
        out["rules"] = match current_rules(ctx.conn, &p.id)? {
            Some((v, r, ex)) => json!({"version": v, "rules": r, "original_text_excerpt": ex}),
            None => json!({"version": null, "rules": null, "note": "no verified structured rules yet; matching reports manual_review"}),
        };
    }
    if inc("sources") || inc("provenance") || inc("changes") || inc("aliases") {
        out["provenance"] = provenance(ctx, &p.id)?;
    }
    if inc("matches") && ctx.actor.has_scope("match.evaluate", None) {
        out["matches"] = json!(query_all(
            ctx.conn,
            "SELECT person_id, assessment, stale, evaluated_at FROM matches WHERE opportunity_id = ?1",
            [&p.id]
        )?);
    }
    Ok(out)
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let q: OpportunityQuery = parse_params(p)?;
    search(ctx, q)
}

fn op_new(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        since_days: Option<u32>,
        #[serde(flatten)]
        query: OpportunityQuery,
    }
    let p: P = parse_params(p)?;
    let days = p.since_days.unwrap_or(14) as i64;
    let since = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() - time::Duration::days(days));
    let mut q = p.query;
    q.sort = Some("updated".into());
    let mut out = search(ctx, q)?;
    if let Some(items) = out["items"].as_array_mut() {
        items.retain(|i| i["first_seen_at"].as_str().unwrap_or("") >= since.as_str());
    }
    out["since"] = json!(since);
    Ok(out)
}

fn op_trending(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let mut q: OpportunityQuery = parse_params(p)?;
    q.sort = Some("activity".into());
    q.only_verified = true;
    let mut out = search(ctx, q)?;
    out["ranking"] = json!("deterministic: most recent official update first, then upcoming deadline");
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
pub struct CompareParams {
    pub ids: Vec<String>,
}

fn op_compare(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: CompareParams = parse_params(p)?;
    if p.ids.len() < 2 || p.ids.len() > 6 {
        return Err(OcError::invalid("compare 2 to 6 opportunities"));
    }
    let mut rows = Vec::new();
    for id in &p.ids {
        let mut r = load(ctx, id)?;
        r["rules_version"] = json!(current_rules(ctx.conn, id)?.map(|r| r.0));
        rows.push(r);
    }
    let fields = [
        "title",
        "status",
        "verification_state",
        "claim_deadline",
        "deadline_verified",
        "class_period_start",
        "class_period_end",
        "administrator",
        "claim_method",
        "proof_required",
        "official_url",
        "claim_url",
        "benefit",
        "rules_version",
    ];
    let table: Vec<Value> = fields
        .iter()
        .map(|f| json!({"field": f, "values": rows.iter().map(|r| r[*f].clone()).collect::<Vec<_>>()}))
        .collect();
    Ok(json!({"ids": p.ids, "comparison": table}))
}

fn op_history(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        days: Option<u32>,
        #[serde(flatten)]
        page: PageParams,
    }
    let p: P = parse_params(p)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let since = openclaim_domain::dates::format_ts(time::OffsetDateTime::now_utc() - time::Duration::days(p.days.unwrap_or(30) as i64));
    let rows = query_all(
        ctx.conn,
        &format!(
            "SELECT c.opportunity_id, o.title, c.field, c.old_value, c.new_value, c.detected_at FROM opportunity_changes c JOIN opportunities o ON o.id = c.opportunity_id WHERE c.detected_at >= ?1 AND c.detected_at <= ?2 ORDER BY c.detected_at DESC{}",
            pager.sql_limit()
        ),
        (&since, &pager.snapshot),
    )?;
    Ok(pager.wrap(rows, None))
}

fn op_watch(ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    let seq: i64 = scalar(ctx.conn, "SELECT COALESCE(max(seq),0) FROM events", [])?.unwrap_or(0);
    Ok(
        json!({"after_seq": seq, "topics": ["opportunity.new", "opportunity.changed", "source.failed", "alert.due"], "next_action": "poll events.poll with after_seq, or run `openclaim discover watch` which streams NDJSON"}),
    )
}

fn op_rules(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    load(ctx, &p.id)?;
    let history = query_all(
        ctx.conn,
        "SELECT version, curated_by, created_at, original_text_excerpt FROM opportunity_rules WHERE opportunity_id = ?1 ORDER BY version DESC",
        [&p.id],
    )?;
    Ok(match current_rules(ctx.conn, &p.id)? {
        Some((v, r, ex)) => json!({"opportunity_id": p.id, "version": v, "rules": r, "original_text_excerpt": ex, "history": history}),
        None => {
            json!({"opportunity_id": p.id, "version": null, "rules": null, "history": history, "note": "no verified structured rules; add them with opportunities.set_rules citing the official text"})
        }
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct SetRulesParams {
    pub opportunity_id: String,
    pub rules: RuleSet,
    /// Verbatim official text the rules were derived from.
    pub original_text_excerpt: String,
    /// Evidence link id (from provenance) that holds the text, if any.
    #[serde(default)]
    pub evidence_id: Option<String>,
}

fn op_set_rules(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SetRulesParams = parse_params(p)?;
    load(ctx, &p.opportunity_id)?;
    p.rules.validate()?;
    if p.original_text_excerpt.trim().len() < 20 {
        return Err(OcError::invalid("original_text_excerpt must quote the official eligibility text"));
    }
    let version: i64 = scalar::<i64, _>(
        ctx.conn,
        "SELECT COALESCE(max(version),0) FROM opportunity_rules WHERE opportunity_id = ?1",
        [&p.opportunity_id],
    )?
    .unwrap_or(0)
        + 1;
    exec(
        ctx.conn,
        "INSERT INTO opportunity_rules(opportunity_id, version, criteria_ast_json, original_text_excerpt, evidence_id, curated_by, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        (
            &p.opportunity_id,
            version,
            serde_json::to_string(&p.rules)?,
            &p.original_text_excerpt,
            &p.evidence_id,
            ctx.actor_id(),
            &ctx.now,
        ),
    )?;
    exec(
        ctx.conn,
        "UPDATE opportunities SET version = version + 1, updated_at = ?2 WHERE id = ?1",
        (&p.opportunity_id, &ctx.now),
    )?;
    exec(ctx.conn, "UPDATE matches SET stale = 1 WHERE opportunity_id = ?1", [&p.opportunity_id])?;
    crate::jobs::enqueue(ctx, "match_refresh", json!({"opportunity_id": p.opportunity_id}), None)?;
    ctx.emit("opportunity.rules_changed", json!({"opportunity_id": p.opportunity_id, "version": version}))?;
    Ok(json!({"opportunity_id": p.opportunity_id, "rules_version": version}))
}

fn op_sources(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    load(ctx, &p.id)?;
    provenance(ctx, &p.id)
}

fn op_changes(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    load(ctx, &p.id)?;
    let rows = query_all(
        ctx.conn,
        "SELECT id, field, old_value, new_value, snapshot_id, detected_at, reviewed_at FROM opportunity_changes WHERE opportunity_id = ?1 ORDER BY detected_at DESC",
        [&p.id],
    )?;
    Ok(json!({"opportunity_id": p.id, "changes": rows}))
}

#[derive(Deserialize, JsonSchema)]
pub struct VerifyParams {
    pub id: String,
    /// verified | disputed | verification_pending
    pub state: String,
    pub note: String,
}

fn op_verify(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: VerifyParams = parse_params(p)?;
    let state = VerificationState::parse(&p.state).ok_or_else(|| OcError::invalid("state must be verified, disputed or verification_pending"))?;
    load(ctx, &p.id)?;
    if state == VerificationState::Verified {
        let has_primary: i64 = scalar(ctx.conn, "SELECT count(*) FROM evidence_links WHERE opportunity_id = ?1", [&p.id])?.unwrap_or(0);
        if has_primary == 0 {
            return Err(OcError::policy("cannot mark verified without at least one evidence link to a source snapshot"));
        }
    }
    exec(
        ctx.conn,
        "UPDATE opportunities SET verification_state = ?2, verified_at = CASE WHEN ?2 = 'verified' THEN ?3 ELSE verified_at END, version = version + 1, updated_at = ?3 WHERE id = ?1",
        (&p.id, state.as_str(), &ctx.now),
    )?;
    exec(
        ctx.conn,
        "UPDATE opportunity_changes SET reviewed_at = ?2 WHERE opportunity_id = ?1 AND reviewed_at IS NULL",
        (&p.id, &ctx.now),
    )?;
    ctx.audit("opportunities.verify", &p.id, json!({"state": p.state, "note": p.note}))?;
    Ok(json!({"id": p.id, "verification_state": state.as_str()}))
}

#[derive(Deserialize, JsonSchema)]
pub struct MarkParams {
    pub id: String,
    #[serde(default = "default_true")]
    pub on: bool,
}

fn default_true() -> bool {
    true
}

fn set_mark(ctx: &mut Ctx<'_>, p: Value, col: &str) -> OcResult<Value> {
    let p: MarkParams = parse_params(p)?;
    load(ctx, &p.id)?;
    exec(
        ctx.conn,
        &format!(
            "INSERT INTO opportunity_marks(opportunity_id, {col}, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(opportunity_id) DO UPDATE SET {col} = excluded.{col}, updated_at = excluded.updated_at"
        ),
        (&p.id, p.on as i32, &ctx.now),
    )?;
    if col == "bookmarked" && p.on {
        crate::modules::alerts::schedule_deadline_alerts(ctx.conn, "opportunity", &p.id, &ctx.now)?;
    }
    Ok(json!({"id": p.id, col: p.on}))
}

fn op_bookmark(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    set_mark(ctx, p, "bookmarked")
}

fn op_hide(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    set_mark(ctx, p, "hidden")
}

#[derive(Deserialize, JsonSchema)]
pub struct ReportParams {
    pub id: String,
    pub reason: String,
}

fn op_report(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ReportParams = parse_params(p)?;
    load(ctx, &p.id)?;
    let rid = openclaim_db::new_id();
    exec(
        ctx.conn,
        "INSERT INTO opportunity_reports(id, opportunity_id, reason, reported_by, created_at) VALUES (?1,?2,?3,?4,?5)",
        (&rid, &p.id, openclaim_security::sanitize::clean_text(&p.reason, 2000), ctx.actor_id(), &ctx.now),
    )?;
    exec(
        ctx.conn,
        "UPDATE opportunities SET verification_state = CASE WHEN verification_state = 'verified' THEN 'disputed' ELSE verification_state END WHERE id = ?1",
        [&p.id],
    )?;
    Ok(json!({"report_id": rid, "note": "the record is shown as disputed until the owner re-verifies it"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ExportParams {
    #[serde(flatten)]
    pub query: OpportunityQuery,
    /// json | csv | markdown
    #[serde(default)]
    pub format: Option<String>,
}

pub fn to_csv(items: &[Value], columns: &[&str]) -> String {
    let esc = |v: &Value| -> String {
        let s = match v {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        format!("\"{}\"", s.replace('"', "\"\""))
    };
    let mut out = columns.join(",") + "\n";
    for it in items {
        out.push_str(&columns.iter().map(|c| esc(&it[*c])).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

pub fn to_markdown(items: &[Value], columns: &[&str]) -> String {
    let mut out = format!(
        "| {} |\n|{}|\n",
        columns.join(" | "),
        columns.iter().map(|_| "---").collect::<Vec<_>>().join("|")
    );
    for it in items {
        let cells: Vec<String> = columns
            .iter()
            .map(|c| match &it[*c] {
                Value::Null => String::new(),
                Value::String(s) => s.replace('|', "\\|"),
                o => o.to_string(),
            })
            .collect();
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    out
}

fn op_export(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ExportParams = parse_params(p)?;
    let mut q = p.query;
    q.page.limit = Some(500);
    let res = search(ctx, q)?;
    let items = res["items"].as_array().cloned().unwrap_or_default();
    let cols = [
        "id",
        "title",
        "status",
        "verification_state",
        "claim_deadline",
        "deadline_verified",
        "administrator",
        "official_url",
        "claim_url",
        "source_id",
        "verified_at",
    ];
    Ok(match p.format.as_deref().unwrap_or("json") {
        "csv" => json!({"format": "csv", "content": to_csv(&items, &cols), "count": items.len()}),
        "markdown" => json!({"format": "markdown", "content": to_markdown(&items, &cols), "count": items.len()}),
        _ => json!({"format": "json", "items": items, "count": items.len()}),
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct TipParams {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

fn op_tip_submit(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: TipParams = parse_params(p)?;
    if !openclaim_security::netpolicy::is_safe_display_link(&p.url) {
        return Err(OcError::invalid("tip URL must be an http(s) link to a named host"));
    }
    let id = openclaim_db::new_id();
    exec(
        ctx.conn,
        "INSERT INTO tips(id, url, title, note, submitted_by, state, created_at) VALUES (?1,?2,?3,?4,?5,'quarantined',?6)",
        (
            &id,
            &p.url,
            p.title.map(|t| openclaim_security::sanitize::clean_text(&t, 300)),
            p.note.map(|n| openclaim_security::sanitize::clean_text(&n, 2000)),
            ctx.actor_id(),
            &ctx.now,
        ),
    )?;
    Ok(json!({"tip_id": id, "state": "quarantined", "note": "tips stay quarantined until a primary source corroborates them"}))
}

fn op_tip_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PageParams = parse_params(p)?;
    let pager = Pager::from(&p, &ctx.now)?;
    let rows = query_all(
        ctx.conn,
        &format!(
            "SELECT id, url, title, note, submitted_by, state, opportunity_id, created_at FROM tips WHERE created_at <= ?1 ORDER BY created_at DESC{}",
            pager.sql_limit()
        ),
        [&pager.snapshot],
    )?;
    Ok(pager.wrap(rows, None))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!(
            "opportunities.list",
            "opportunities.read",
            Read,
            "Search and filter the catalog (typed filters or `filter` string)",
            OpportunityQuery,
            op_list,
            mcp = "discover_opportunities"
        ),
        op!(
            "discover.search",
            "discover.read",
            Read,
            "Search the catalog (alias of opportunities.list)",
            OpportunityQuery,
            op_list
        ),
        op!(
            "discover.new",
            "discover.read",
            Read,
            "Opportunities first seen in the last N days",
            OpportunityQuery,
            op_new
        ),
        op!(
            "discover.trending",
            "discover.read",
            Read,
            "Verified opportunities by recent official activity (deterministic)",
            OpportunityQuery,
            op_trending
        ),
        op!(
            "discover.compare",
            "opportunities.read",
            Read,
            "Side-by-side comparison of opportunities",
            CompareParams,
            op_compare,
            mcp = "compare_opportunities"
        ),
        op!(
            "discover.history",
            "discover.read",
            Read,
            "Recent official changes across the catalog",
            PageParams,
            op_history
        ),
        op!(
            "discover.watch",
            "discover.read",
            Read,
            "Event cursor and topics for live catalog changes",
            super::Empty,
            op_watch
        ),
        op!(
            "opportunities.show",
            "opportunities.read",
            Read,
            "Full opportunity with optional rules, provenance and changes",
            ShowParams,
            op_show,
            resource = "id",
            mcp = "get_opportunity"
        ),
        op!(
            "opportunities.rules",
            "opportunities.read",
            Read,
            "Structured eligibility rules with source citations",
            IdParam,
            op_rules,
            mcp = "get_opportunity_rules"
        ),
        op!(
            "opportunities.set_rules",
            "opportunities.write",
            Elevated,
            "Curate structured rules from quoted official text",
            SetRulesParams,
            op_set_rules
        ),
        op!(
            "opportunities.sources",
            "opportunities.read",
            Read,
            "Evidence links and snapshots behind a record",
            IdParam,
            op_sources
        ),
        op!(
            "opportunities.changes",
            "opportunities.read",
            Read,
            "Field-level change history",
            IdParam,
            op_changes
        ),
        op!(
            "opportunities.verify",
            "opportunities.write",
            Elevated,
            "Owner review: mark verified, disputed or pending",
            VerifyParams,
            op_verify
        ),
        op!(
            "opportunities.bookmark",
            "opportunities.write",
            Write,
            "Bookmark (schedules deadline reminders)",
            MarkParams,
            op_bookmark
        ),
        op!(
            "opportunities.hide",
            "opportunities.write",
            Write,
            "Hide from default listings",
            MarkParams,
            op_hide
        ),
        op!(
            "opportunities.report",
            "opportunities.write",
            Write,
            "Report an inaccurate record",
            ReportParams,
            op_report
        ),
        op!(
            "opportunities.export",
            "opportunities.read",
            Read,
            "Export search results as json, csv or markdown",
            ExportParams,
            op_export
        ),
        op!(
            "tips.submit",
            "opportunities.write",
            Write,
            "Submit a settlement tip URL (quarantined lead)",
            TipParams,
            op_tip_submit
        ),
        op!(
            "tips.list",
            "opportunities.read",
            Read,
            "List quarantined and corroborated tips",
            PageParams,
            op_tip_list
        ),
    ]
}
