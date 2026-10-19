//! Deterministic eligibility matching over human-confirmed facts (MDR §5.2, §9).

use super::parse_params;
use crate::ctx::Ctx;
use crate::modules::people::ensure_person;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require};
use openclaim_db::rusqlite::Connection;
use openclaim_domain::rules::{Assessment, ConsumerFactRecord, FactSet, MatchState, ResidenceFact, evaluate};
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn fact_set(conn: &Connection, person_id: &str, opportunity_id: &str) -> OcResult<FactSet> {
    let residences = query_all(
        conn,
        "SELECT id, jurisdiction, start_date, end_date FROM residence_periods WHERE person_id = ?1",
        [person_id],
    )?
    .into_iter()
    .map(|r| ResidenceFact {
        id: r["id"].as_str().unwrap_or("").into(),
        jurisdiction: r["jurisdiction"].as_str().unwrap_or("").into(),
        start: r["start_date"].as_str().map(str::to_string),
        end: r["end_date"].as_str().map(str::to_string),
    })
    .collect();
    let consumer_facts = query_all(
        conn,
        "SELECT id, fact_type, merchant, product, start_date, end_date, positive FROM consumer_facts WHERE person_id = ?1 AND asserted_by_user = 1",
        [person_id],
    )?
    .into_iter()
    .map(|r| ConsumerFactRecord {
        id: r["id"].as_str().unwrap_or("").into(),
        fact_type: r["fact_type"].as_str().unwrap_or("").into(),
        merchant: r["merchant"].as_str().map(str::to_string),
        product: r["product"].as_str().map(str::to_string),
        start: r["start_date"].as_str().map(str::to_string),
        end: r["end_date"].as_str().map(str::to_string),
        positive: r["positive"].as_bool().unwrap_or(true),
    })
    .collect();
    let attestations = query_all(
        conn,
        "SELECT key, value FROM attestations WHERE person_id = ?1 AND opportunity_id = ?2",
        (person_id, opportunity_id),
    )?
    .into_iter()
    .map(|r| (r["key"].as_str().unwrap_or("").to_string(), r["value"].as_i64().unwrap_or(0) != 0))
    .collect();
    let not_in_class: Option<String> = crate::sql::scalar(
        conn,
        "SELECT recorded_at FROM class_exclusions WHERE person_id = ?1 AND opportunity_id = ?2",
        (person_id, opportunity_id),
    )?;
    Ok(FactSet {
        residences,
        consumer_facts,
        attestations,
        not_in_class: not_in_class.is_some(),
    })
}

/// Evaluates and persists one (person, opportunity) pair.
pub fn evaluate_pair(conn: &Connection, person_id: &str, opportunity_id: &str, now: &str) -> OcResult<(Assessment, Option<i64>)> {
    let rules = crate::modules::opportunities::current_rules(conn, opportunity_id)?;
    let facts = fact_set(conn, person_id, opportunity_id)?;
    let assessment = evaluate(rules.as_ref().map(|r| &r.1), &facts)?;
    let version = rules.as_ref().map(|r| r.0);
    let prev: Option<String> = crate::sql::scalar(
        conn,
        "SELECT assessment FROM matches WHERE person_id = ?1 AND opportunity_id = ?2",
        (person_id, opportunity_id),
    )?;
    exec(
        conn,
        "INSERT INTO matches(person_id, opportunity_id, assessment, reasons_json, rules_version, stale, evaluated_at) VALUES (?1,?2,?3,?4,?5,0,?6)
         ON CONFLICT(person_id, opportunity_id) DO UPDATE SET assessment = excluded.assessment, reasons_json = excluded.reasons_json, rules_version = excluded.rules_version, stale = 0, evaluated_at = excluded.evaluated_at",
        (person_id, opportunity_id, assessment.state.as_str(), serde_json::to_string(&assessment)?, version, now),
    )?;
    if prev.as_deref() != Some(assessment.state.as_str()) {
        crate::events::emit(
            conn,
            "match.changed",
            json!({"person_id": person_id, "opportunity_id": opportunity_id, "state": assessment.state.as_str()}),
        )?;
        if assessment.state == MatchState::PotentialMatch {
            crate::modules::alerts::schedule_deadline_alerts(conn, "opportunity", opportunity_id, now)?;
        }
    }
    Ok((assessment, version))
}

pub fn evaluate_person(conn: &Connection, person_id: &str, only_stale: bool, now: &str) -> OcResult<Vec<(String, MatchState)>> {
    let ids = if only_stale {
        query_all(
            conn,
            "SELECT o.id FROM opportunities o LEFT JOIN matches m ON m.opportunity_id = o.id AND m.person_id = ?1 WHERE o.verification_state IN ('verified','stale') AND o.archived_at IS NULL AND (m.person_id IS NULL OR m.stale = 1)",
            [person_id],
        )?
    } else {
        query_all(
            conn,
            "SELECT id FROM opportunities WHERE verification_state IN ('verified','stale') AND archived_at IS NULL",
            [],
        )?
    };
    let mut out = Vec::new();
    for row in ids {
        let oid = row["id"].as_str().unwrap_or("").to_string();
        let (a, _) = evaluate_pair(conn, person_id, &oid, now)?;
        out.push((oid, a.state));
    }
    Ok(out)
}

fn agent_safe(ctx: &Ctx<'_>, person_id: &str, a: &Assessment) -> Value {
    if ctx.actor.is_owner() || ctx.actor.has_scope("people.read", Some(person_id)) {
        return json!(a);
    }
    json!({
        "state": a.state,
        "summary": a.summary,
        "reasons": a.reasons.iter().map(|r| json!({"rule_id": r.rule_id, "label": r.label, "required": r.required, "outcome": r.outcome, "source_excerpt": r.source_excerpt, "source_locator": r.source_locator})).collect::<Vec<_>>(),
        "missing_facts_count": a.missing_facts.len(),
        "disclaimer": a.disclaimer,
        "redacted": "explanations referencing personal facts need scope people.read",
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct EvaluateParams {
    pub person_id: String,
    /// Omit to evaluate every verified opportunity.
    #[serde(default)]
    pub opportunity_id: Option<String>,
}

fn op_evaluate(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: EvaluateParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    match &p.opportunity_id {
        Some(oid) => {
            require(query_one(ctx.conn, "SELECT id FROM opportunities WHERE id = ?1", [oid])?, "opportunity", oid)?;
            let (a, v) = evaluate_pair(ctx.conn, &p.person_id, oid, &ctx.now)?;
            Ok(
                json!({"person_id": p.person_id, "opportunity_id": oid, "rules_version": v, "assessment": agent_safe(ctx, &p.person_id, &a), "trust": "deterministic_evaluation"}),
            )
        }
        None => {
            let results = evaluate_person(ctx.conn, &p.person_id, false, &ctx.now)?;
            let mut counts = serde_json::Map::new();
            for (_, s) in &results {
                *counts.entry(s.as_str()).or_insert(json!(0)) = json!(counts.get(s.as_str()).and_then(|v| v.as_i64()).unwrap_or(0) + 1);
            }
            Ok(
                json!({"person_id": p.person_id, "evaluated": results.len(), "counts": counts, "results": results.iter().map(|(o, s)| json!({"opportunity_id": o, "state": s})).collect::<Vec<_>>(), "trust": "deterministic_evaluation"}),
            )
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct PairParams {
    pub person_id: String,
    pub opportunity_id: String,
}

fn op_explain(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PairParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let opp = require(
        query_one(
            ctx.conn,
            "SELECT id, title, official_url, claim_deadline, deadline_verified, verification_state FROM opportunities WHERE id = ?1",
            [&p.opportunity_id],
        )?,
        "opportunity",
        &p.opportunity_id,
    )?;
    let (a, v) = evaluate_pair(ctx.conn, &p.person_id, &p.opportunity_id, &ctx.now)?;
    Ok(
        json!({"person_id": p.person_id, "opportunity": opp, "rules_version": v, "assessment": agent_safe(ctx, &p.person_id, &a), "confirm_controls": {"attest": "facts.attest", "not_in_class": "facts.exclude", "add_fact": "facts.add"}, "trust": "deterministic_evaluation"}),
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct PersonParams {
    pub person_id: String,
    /// Filter by state (potential_match, needs_more_information, …)
    #[serde(default)]
    pub state: Option<String>,
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PersonParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let rows = query_all(
        ctx.conn,
        "SELECT m.opportunity_id, o.title, o.status, o.claim_deadline, o.deadline_verified, o.verification_state, o.official_url, m.assessment, m.stale, m.rules_version, m.evaluated_at
         FROM matches m JOIN opportunities o ON o.id = m.opportunity_id WHERE m.person_id = ?1 AND o.archived_at IS NULL ORDER BY
         CASE m.assessment WHEN 'potential_match' THEN 0 WHEN 'needs_more_information' THEN 1 WHEN 'manual_review' THEN 2 WHEN 'unlikely' THEN 3 ELSE 4 END,
         CASE WHEN o.claim_deadline IS NULL THEN 1 ELSE 0 END, o.claim_deadline",
        [&p.person_id],
    )?;
    let items: Vec<Value> = rows.into_iter().filter(|r| p.state.as_deref().is_none_or(|s| r["assessment"] == s)).collect();
    Ok(json!({"person_id": p.person_id, "items": items, "trust": "deterministic_evaluation"}))
}

fn op_missing(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PersonParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let rows = query_all(
        ctx.conn,
        "SELECT m.opportunity_id, o.title, m.reasons_json FROM matches m JOIN opportunities o ON o.id = m.opportunity_id WHERE m.person_id = ?1 AND m.assessment = 'needs_more_information'",
        [&p.person_id],
    )?;
    let mut by_question: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    for r in rows {
        if let Some(missing) = r["reasons"]["missing_facts"].as_array() {
            for m in missing {
                by_question
                    .entry(m.as_str().unwrap_or("").to_string())
                    .or_default()
                    .push(json!({"opportunity_id": r["opportunity_id"], "title": r["title"]}));
            }
        }
    }
    let items: Vec<Value> = by_question.into_iter().map(|(q, opps)| json!({"question": q, "unblocks": opps})).collect();
    if !ctx.actor.is_owner() && !ctx.actor.has_scope("people.read", Some(&p.person_id)) {
        return Ok(json!({"person_id": p.person_id, "count": items.len(), "redacted": "questions reference personal facts; need scope people.read"}));
    }
    Ok(json!({"person_id": p.person_id, "items": items}))
}

fn op_refresh(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        person_id: Option<String>,
    }
    let p: P = parse_params(p)?;
    if let Some(pid) = &p.person_id {
        ensure_person(ctx, pid)?;
    }
    let job_id = crate::jobs::enqueue(ctx, "match_refresh", json!({"person_id": p.person_id}), None)?;
    Ok(json!({"job_id": job_id, "status": "queued"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct CompareParams {
    pub person_id: String,
    pub opportunity_ids: Vec<String>,
}

fn op_compare(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: CompareParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    if p.opportunity_ids.len() < 2 || p.opportunity_ids.len() > 6 {
        return Err(OcError::invalid("compare 2 to 6 opportunities"));
    }
    let mut items = Vec::new();
    for oid in &p.opportunity_ids {
        let opp = require(
            query_one(
                ctx.conn,
                "SELECT id, title, claim_deadline, deadline_verified, proof_required, claim_method FROM opportunities WHERE id = ?1",
                [oid],
            )?,
            "opportunity",
            oid,
        )?;
        let (a, _) = evaluate_pair(ctx.conn, &p.person_id, oid, &ctx.now)?;
        items.push(json!({"opportunity": opp, "state": a.state, "missing_count": a.missing_facts.len(), "required_yes": a.reasons.iter().filter(|r| r.required && r.outcome == openclaim_domain::rules::Tri::Yes).count(), "effort": match opp["proof_required"].as_str() { Some("required") => "proof needed", Some("none") => "low", _ => "unknown" }}));
    }
    Ok(json!({"person_id": p.person_id, "items": items, "ranking": "deterministic: match state, then verified deadline proximity, then required effort"}))
}

fn op_export(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: PersonParams = parse_params(p)?;
    let list = op_list(ctx, json!({"person_id": p.person_id, "state": p.state}))?;
    let items = list["items"].as_array().cloned().unwrap_or_default();
    let cols = [
        "opportunity_id",
        "title",
        "assessment",
        "status",
        "claim_deadline",
        "deadline_verified",
        "official_url",
        "evaluated_at",
    ];
    Ok(json!({"format": "csv", "content": crate::modules::opportunities::to_csv(&items, &cols), "count": items.len()}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!(
            "match.evaluate",
            "match.evaluate",
            Write,
            "Deterministic eligibility evaluation; persists results",
            EvaluateParams,
            op_evaluate,
            mcp = "evaluate_match"
        ),
        op!(
            "match.explain",
            "match.evaluate",
            Read,
            "Why a person matched (or not), with cited rules",
            PairParams,
            op_explain
        ),
        op!("match.list", "match.evaluate", Read, "Stored match states for a person", PersonParams, op_list),
        op!(
            "match.missing",
            "match.evaluate",
            Read,
            "Questions that would resolve needs_more_information matches",
            PersonParams,
            op_missing
        ),
        op!("match.refresh", "match.evaluate", Write, "Queue a re-evaluation job", super::Empty, op_refresh),
        op!(
            "match.compare",
            "match.evaluate",
            Read,
            "Compare several opportunities for one person",
            CompareParams,
            op_compare
        ),
        op!("match.export", "match.evaluate", Read, "Export match states as CSV", PersonParams, op_export),
    ]
}
