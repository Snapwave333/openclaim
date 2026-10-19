//! Consumer facts, attestations and the untrusted proposal queue (MDR §5.4).

use super::{IdParam, PageParams, Pager, opt_date, parse_params};
use crate::ctx::Ctx;
use crate::modules::people::ensure_person;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, query_all, query_one, require, scalar};
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FactInput {
    /// purchase, subscription, account, service, vehicle, employment, residence_event, other
    pub fact_type: String,
    #[serde(default)]
    pub merchant: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    /// false records "this did not happen".
    #[serde(default = "default_true")]
    pub positive: bool,
}

fn default_true() -> bool {
    true
}

fn validate_fact(f: &FactInput) -> OcResult<(Option<String>, Option<String>)> {
    if f.fact_type.trim().is_empty() || f.fact_type.len() > 40 {
        return Err(OcError::invalid(
            "fact_type is required (purchase, subscription, account, service, vehicle, employment, other)",
        ));
    }
    let (s, e) = (opt_date(&f.start)?, opt_date(&f.end)?);
    if let (Some(a), Some(b)) = (&s, &e)
        && a > b
    {
        return Err(OcError::invalid("start must not be after end"));
    }
    for t in [&f.merchant, &f.product] {
        if t.as_ref().is_some_and(|t| t.len() > 300) {
            return Err(OcError::invalid("merchant/product must be under 300 characters"));
        }
    }
    Ok((s, e))
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        person_id: String,
        #[serde(flatten)]
        page: PageParams,
    }
    let p: P = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let rows = query_all(
        ctx.conn,
        &format!(
            "SELECT id, fact_type, merchant, product, start_date, end_date, positive, proof_id, source_proposal_id, created_at FROM consumer_facts WHERE person_id = ?1 AND created_at <= ?2 ORDER BY created_at DESC{}",
            pager.sql_limit()
        ),
        (&p.person_id, &pager.snapshot),
    )?;
    let mut out = pager.wrap(rows, None);
    out["trust"] = json!("human_confirmed");
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
pub struct AddParams {
    pub person_id: String,
    #[serde(flatten)]
    pub fact: FactInput,
    #[serde(default)]
    pub proof_id: Option<String>,
}

fn insert_fact(ctx: &Ctx<'_>, person_id: &str, f: &FactInput, proof_id: Option<&str>, proposal_id: Option<&str>) -> OcResult<String> {
    let (s, e) = validate_fact(f)?;
    if let Some(d) = proof_id {
        require(scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE id = ?1", [d])?, "document", d)?;
    }
    let id = openclaim_db::new_id();
    exec(
        ctx.conn,
        "INSERT INTO consumer_facts(id, person_id, fact_type, merchant, product, start_date, end_date, positive, proof_id, asserted_by_user, source_proposal_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?11)",
        (
            &id,
            person_id,
            f.fact_type.trim().to_lowercase(),
            &f.merchant,
            &f.product,
            s,
            e,
            f.positive as i32,
            proof_id,
            proposal_id,
            &ctx.now,
        ),
    )?;
    exec(ctx.conn, "UPDATE matches SET stale = 1 WHERE person_id = ?1", [person_id])?;
    exec(
        ctx.conn,
        "UPDATE people SET version = version + 1, updated_at = ?2 WHERE id = ?1",
        (person_id, &ctx.now),
    )?;
    Ok(id)
}

fn op_add(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: AddParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    if ctx.actor.is_agent() {
        return Err(OcError::denied("agents cannot assert facts; use facts.propose so a human confirms them"));
    }
    let id = insert_fact(ctx, &p.person_id, &p.fact, p.proof_id.as_deref(), None)?;
    crate::jobs::enqueue(ctx, "match_refresh", json!({"person_id": p.person_id}), None)?;
    Ok(json!({"id": id, "person_id": p.person_id, "trust": "human_confirmed"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ProposeParams {
    pub person_id: String,
    #[serde(default)]
    pub document_id: Option<String>,
    pub proposal: FactInput,
    /// Where in the document the fact appears (e.g. "chars 120-180" or "page 2").
    #[serde(default)]
    pub evidence_locator: Option<String>,
}

fn op_propose(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ProposeParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    validate_fact(&p.proposal)?;
    if let Some(d) = &p.document_id {
        require(scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE id = ?1", [d])?, "document", d)?;
    }
    let id = openclaim_db::new_id();
    exec(
        ctx.conn,
        "INSERT INTO proposed_facts(id, person_id, document_id, proposal_json, evidence_locator, agent_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        (
            &id,
            &p.person_id,
            &p.document_id,
            serde_json::to_string(&p.proposal)?,
            p.evidence_locator.as_deref().map(|l| openclaim_security::sanitize::clean_text(l, 300)),
            ctx.actor_id(),
            &ctx.now,
        ),
    )?;
    ctx.emit("fact.proposed", json!({"proposal_id": id, "person_id": p.person_id}))?;
    Ok(json!({"proposal_id": id, "validation_state": "proposed", "trust": "agent_proposed", "note": "not used for matching until a human confirms it"}))
}

fn op_validate(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT * FROM proposed_facts WHERE id = ?1", [&p.id])?, "proposal", &p.id)?;
    let proposal: FactInput = serde_json::from_value(row["proposal"].clone())?;
    let mut problems = Vec::new();
    if let Err(e) = validate_fact(&proposal) {
        problems.push(e.message);
    }
    let mut evidence_found = None;
    if let Some(doc) = row["document_id"].as_str() {
        let body: Option<String> = scalar(ctx.conn, "SELECT body FROM documents_fts WHERE document_id = ?1", [doc])?;
        match body {
            None => problems.push("the document has no extracted text to check against".into()),
            Some(b) => {
                let needle = proposal.merchant.clone().or(proposal.product.clone()).unwrap_or_default().to_lowercase();
                evidence_found = Some(!needle.is_empty() && b.to_lowercase().contains(&needle));
                if evidence_found == Some(false) {
                    problems.push("the merchant/product text does not appear in the document".into());
                }
            }
        }
    } else {
        problems.push("no source document linked; the owner must verify from memory or other proof".into());
    }
    Ok(
        json!({"proposal_id": p.id, "proposal": proposal, "evidence_found": evidence_found, "problems": problems, "ok": problems.is_empty(), "trust": "deterministic_evaluation"}),
    )
}

fn decide(ctx: &mut Ctx<'_>, id: &str, confirm: bool) -> OcResult<Value> {
    let row = require(query_one(ctx.conn, "SELECT * FROM proposed_facts WHERE id = ?1", [id])?, "proposal", id)?;
    if row["validation_state"] != "proposed" {
        return Err(OcError::conflict(format!("proposal is already {}", row["validation_state"])));
    }
    let state = if confirm { "confirmed" } else { "rejected" };
    exec(
        ctx.conn,
        "UPDATE proposed_facts SET validation_state = ?2, decided_by = ?3, decided_at = ?4 WHERE id = ?1",
        (id, state, ctx.actor_id(), &ctx.now),
    )?;
    let mut fact_id = None;
    if confirm {
        let proposal: FactInput = serde_json::from_value(row["proposal"].clone())?;
        let pid = row["person_id"].as_str().unwrap_or("").to_string();
        fact_id = Some(insert_fact(ctx, &pid, &proposal, row["document_id"].as_str(), Some(id))?);
        crate::jobs::enqueue(ctx, "match_refresh", json!({"person_id": pid}), None)?;
    }
    Ok(json!({"proposal_id": id, "validation_state": state, "fact_id": fact_id}))
}

fn op_confirm(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    decide(ctx, &p.id, true)
}

fn op_reject(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    decide(ctx, &p.id, false)
}

#[derive(Deserialize, JsonSchema)]
pub struct LinkProofParams {
    pub fact_id: String,
    pub document_id: String,
}

fn op_link_proof(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: LinkProofParams = parse_params(p)?;
    require(
        scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE id = ?1", [&p.document_id])?,
        "document",
        &p.document_id,
    )?;
    let n = exec(ctx.conn, "UPDATE consumer_facts SET proof_id = ?2 WHERE id = ?1", (&p.fact_id, &p.document_id))?;
    if n == 0 {
        return Err(OcError::not_found(format!("fact '{}' not found", p.fact_id)));
    }
    exec(
        ctx.conn,
        "INSERT OR REPLACE INTO document_links(document_id, target_kind, target_id, locator, validated, created_at) VALUES (?1,'fact',?2,NULL,1,?3)",
        (&p.document_id, &p.fact_id, &ctx.now),
    )?;
    Ok(json!({"fact_id": p.fact_id, "proof_id": p.document_id}))
}

fn op_remove(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let pid: String = require(scalar(ctx.conn, "SELECT person_id FROM consumer_facts WHERE id = ?1", [&p.id])?, "fact", &p.id)?;
    exec(ctx.conn, "DELETE FROM consumer_facts WHERE id = ?1", [&p.id])?;
    exec(ctx.conn, "UPDATE matches SET stale = 1 WHERE person_id = ?1", [&pid])?;
    Ok(json!({"removed": p.id}))
}

#[derive(Deserialize, JsonSchema)]
pub struct AttestParams {
    pub person_id: String,
    pub opportunity_id: String,
    pub key: String,
    pub value: bool,
}

fn op_attest(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: AttestParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    if ctx.actor.is_agent() {
        return Err(OcError::denied("only the claimant can answer attestation questions"));
    }
    exec(
        ctx.conn,
        "INSERT INTO attestations(person_id, opportunity_id, key, value, recorded_at) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(person_id, opportunity_id, key) DO UPDATE SET value = excluded.value, recorded_at = excluded.recorded_at",
        (&p.person_id, &p.opportunity_id, &p.key, p.value as i32, &ctx.now),
    )?;
    exec(
        ctx.conn,
        "UPDATE matches SET stale = 1 WHERE person_id = ?1 AND opportunity_id = ?2",
        (&p.person_id, &p.opportunity_id),
    )?;
    crate::jobs::enqueue(
        ctx,
        "match_refresh",
        json!({"person_id": p.person_id, "opportunity_id": p.opportunity_id}),
        None,
    )?;
    Ok(json!({"person_id": p.person_id, "opportunity_id": p.opportunity_id, "key": p.key, "value": p.value}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ExcludeParams {
    pub person_id: String,
    pub opportunity_id: String,
    /// true = "I was not part of this class"; false clears it.
    pub not_in_class: bool,
}

fn op_exclude(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ExcludeParams = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    if ctx.actor.is_agent() {
        return Err(OcError::denied("only the claimant can confirm they are not part of a class"));
    }
    if p.not_in_class {
        exec(
            ctx.conn,
            "INSERT OR REPLACE INTO class_exclusions(person_id, opportunity_id, recorded_at) VALUES (?1,?2,?3)",
            (&p.person_id, &p.opportunity_id, &ctx.now),
        )?;
    } else {
        exec(
            ctx.conn,
            "DELETE FROM class_exclusions WHERE person_id = ?1 AND opportunity_id = ?2",
            (&p.person_id, &p.opportunity_id),
        )?;
    }
    crate::jobs::enqueue(
        ctx,
        "match_refresh",
        json!({"person_id": p.person_id, "opportunity_id": p.opportunity_id}),
        None,
    )?;
    Ok(json!({"person_id": p.person_id, "opportunity_id": p.opportunity_id, "not_in_class": p.not_in_class}))
}

fn op_history(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        person_id: String,
        /// proposed | confirmed | rejected
        #[serde(default)]
        state: Option<String>,
    }
    let p: P = parse_params(p)?;
    ensure_person(ctx, &p.person_id)?;
    let rows = match p.state {
        Some(s) => query_all(
            ctx.conn,
            "SELECT id, document_id, proposal_json, evidence_locator, agent_id, validation_state, decided_by, decided_at, created_at FROM proposed_facts WHERE person_id = ?1 AND validation_state = ?2 ORDER BY created_at DESC",
            (&p.person_id, s),
        )?,
        None => query_all(
            ctx.conn,
            "SELECT id, document_id, proposal_json, evidence_locator, agent_id, validation_state, decided_by, decided_at, created_at FROM proposed_facts WHERE person_id = ?1 ORDER BY created_at DESC",
            [&p.person_id],
        )?,
    };
    let attestations = query_all(
        ctx.conn,
        "SELECT opportunity_id, key, value, recorded_at FROM attestations WHERE person_id = ?1",
        [&p.person_id],
    )?;
    Ok(json!({"proposals": rows, "attestations": attestations, "trust": {"proposals": "agent_proposed", "attestations": "human_confirmed"}}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!(
            "facts.list",
            "people.read",
            Read,
            "Human-confirmed consumer facts for a person",
            PageParams,
            op_list
        ),
        op!(
            "facts.add",
            "people.modify",
            Elevated,
            "Assert a fact as the claimant (humans only)",
            AddParams,
            op_add
        ),
        op!(
            "facts.propose",
            "facts.propose",
            Write,
            "Propose a fact for human review (never used until confirmed)",
            ProposeParams,
            op_propose,
            mcp = "propose_fact"
        ),
        op!(
            "facts.validate",
            "facts.propose",
            Read,
            "Deterministic checks of a proposal against its document",
            IdParam,
            op_validate
        ),
        op!(
            "facts.confirm",
            "facts.review",
            Elevated,
            "Confirm a proposal, creating a human-confirmed fact",
            IdParam,
            op_confirm
        ),
        op!("facts.reject", "facts.review", Elevated, "Reject a proposal", IdParam, op_reject),
        op!(
            "facts.link_proof",
            "people.modify",
            Elevated,
            "Link a vault document as proof of a fact",
            LinkProofParams,
            op_link_proof
        ),
        op!("facts.remove", "people.modify", Elevated, "Remove a confirmed fact", IdParam, op_remove),
        op!(
            "facts.attest",
            "people.modify",
            Elevated,
            "Answer a yes/no eligibility question for an opportunity",
            AttestParams,
            op_attest
        ),
        op!(
            "facts.exclude",
            "people.modify",
            Elevated,
            "Record that the person is not part of a class",
            ExcludeParams,
            op_exclude
        ),
        op!(
            "facts.history",
            "people.read",
            Read,
            "Proposals and attestations for a person",
            super::Empty,
            op_history
        ),
    ]
}
