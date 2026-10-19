//! Deterministic eligibility rules engine (MDR §5.2, §9).
//!
//! Every condition evaluates to `yes`, `no` or `unknown`. Only human-confirmed
//! facts are evaluated; agent-proposed facts never reach this module. The
//! output is a *possible* match with cited reasons, never legal eligibility.

use crate::dates::{Span, parse_date};
use crate::error::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tri {
    Yes,
    No,
    Unknown,
}

impl Tri {
    fn not(self) -> Tri {
        match self {
            Tri::Yes => Tri::No,
            Tri::No => Tri::Yes,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Interval {
    /// YYYY-MM-DD, inclusive. Absent means open-ended.
    pub start: Option<String>,
    /// YYYY-MM-DD, inclusive. Absent means open-ended.
    pub end: Option<String>,
}

impl Interval {
    fn span(&self) -> OcResult<Span> {
        Ok(Span {
            start: self.start.as_deref().map(parse_date).transpose()?,
            end: self.end.as_deref().map(parse_date).transpose()?,
        })
    }
}

/// A node of the criteria AST.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Criterion {
    All {
        of: Vec<Criterion>,
    },
    Any {
        of: Vec<Criterion>,
    },
    Not {
        of: Box<Criterion>,
    },
    /// Lived in one of the jurisdictions (USPS state codes or country codes) at some point in `during`.
    ResidedIn {
        jurisdictions: Vec<String>,
        during: Option<Interval>,
    },
    /// Had a purchase/service/account relationship matching the filters during `during`.
    ConsumerFact {
        fact_type: String,
        merchant: Option<String>,
        product: Option<String>,
        during: Option<Interval>,
    },
    /// A yes/no statement only the claimant can make (e.g. "received a notice postcard").
    Attestation {
        key: String,
        question: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    pub id: String,
    pub label: String,
    /// Required (hard) conditions decide `unlikely`/`needs_more_information`.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Ambiguous or complex official wording forces manual review.
    #[serde(default)]
    pub ambiguous: bool,
    pub criterion: Criterion,
    /// Quoted or paraphrased official text the rule was built from.
    pub source_excerpt: String,
    /// URL or locator of the primary source span.
    pub source_locator: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    /// "person" or "household" when the official terms limit claims.
    pub one_claim_per: Option<String>,
}

impl RuleSet {
    pub fn validate(&self) -> OcResult<()> {
        if self.rules.is_empty() {
            return Err(OcError::invalid("a rule set needs at least one rule"));
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.rules {
            if !seen.insert(&r.id) {
                return Err(OcError::invalid(format!("duplicate rule id '{}'", r.id)));
            }
            if r.source_excerpt.trim().is_empty() || r.source_locator.trim().is_empty() {
                return Err(OcError::invalid(format!(
                    "rule '{}' must cite the official text (source_excerpt and source_locator)",
                    r.id
                )));
            }
            validate_criterion(&r.criterion)?;
        }
        Ok(())
    }
}

fn validate_criterion(c: &Criterion) -> OcResult<()> {
    match c {
        Criterion::All { of } | Criterion::Any { of } => {
            if of.is_empty() {
                return Err(OcError::invalid("all/any nodes need at least one child"));
            }
            of.iter().try_for_each(validate_criterion)
        }
        Criterion::Not { of } => validate_criterion(of),
        Criterion::ResidedIn { jurisdictions, during } => {
            if jurisdictions.is_empty() {
                return Err(OcError::invalid("resided_in needs at least one jurisdiction"));
            }
            if let Some(d) = during {
                d.span()?;
            }
            Ok(())
        }
        Criterion::ConsumerFact { fact_type, during, .. } => {
            if fact_type.trim().is_empty() {
                return Err(OcError::invalid("consumer_fact needs a fact_type"));
            }
            if let Some(d) = during {
                d.span()?;
            }
            Ok(())
        }
        Criterion::Attestation { key, .. } => {
            if key.trim().is_empty() {
                return Err(OcError::invalid("attestation needs a key"));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Facts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResidenceFact {
    pub id: String,
    pub jurisdiction: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsumerFactRecord {
    pub id: String,
    pub fact_type: String,
    pub merchant: Option<String>,
    pub product: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    /// `false` records an explicit "this did not happen" statement.
    #[serde(default = "default_true")]
    pub positive: bool,
}

/// Human-confirmed facts for one person.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct FactSet {
    pub residences: Vec<ResidenceFact>,
    pub consumer_facts: Vec<ConsumerFactRecord>,
    pub attestations: BTreeMap<String, bool>,
    /// The user confirmed they are not part of this class.
    #[serde(default)]
    pub not_in_class: bool,
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchState {
    PotentialMatch,
    NeedsMoreInformation,
    Unlikely,
    NotApplicable,
    ManualReview,
}

impl MatchState {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchState::PotentialMatch => "potential_match",
            MatchState::NeedsMoreInformation => "needs_more_information",
            MatchState::Unlikely => "unlikely",
            MatchState::NotApplicable => "not_applicable",
            MatchState::ManualReview => "manual_review",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [
            MatchState::PotentialMatch,
            MatchState::NeedsMoreInformation,
            MatchState::Unlikely,
            MatchState::NotApplicable,
            MatchState::ManualReview,
        ]
        .into_iter()
        .find(|m| m.as_str() == s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuleReason {
    pub rule_id: String,
    pub label: String,
    pub required: bool,
    pub outcome: Tri,
    pub explanation: String,
    pub source_excerpt: String,
    pub source_locator: String,
    pub supporting_fact_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Assessment {
    pub state: MatchState,
    pub summary: String,
    pub reasons: Vec<RuleReason>,
    /// Questions or facts that would resolve unknown conditions.
    pub missing_facts: Vec<String>,
    /// Always present: a suggestion is not legal eligibility.
    pub disclaimer: String,
}

pub const DISCLAIMER: &str = "This is a possible match computed from your locally stored facts. Only the official administrator decides eligibility; review the official class definition before filing.";

struct Eval {
    outcome: Tri,
    explanation: Vec<String>,
    facts: Vec<String>,
    missing: Vec<String>,
}

impl Eval {
    fn leaf(outcome: Tri, explanation: String, facts: Vec<String>, missing: Option<String>) -> Eval {
        Eval {
            outcome,
            explanation: vec![explanation],
            facts,
            missing: missing.into_iter().collect(),
        }
    }
}

fn spans_of(start: &Option<String>, end: &Option<String>) -> OcResult<Span> {
    Ok(Span {
        start: start.as_deref().map(parse_date).transpose()?,
        end: end.as_deref().map(parse_date).transpose()?,
    })
}

/// True when the union of `periods` covers all of `target` (requires bounded target).
fn covers(target: Span, mut periods: Vec<Span>) -> bool {
    let (Some(ts), Some(te)) = (target.start, target.end) else {
        return false;
    };
    periods.sort_by_key(|p| p.start);
    let mut cursor = ts;
    for p in periods {
        let ps = p.start.unwrap_or(time::Date::MIN);
        if ps > cursor {
            return false;
        }
        let pe = p.end.unwrap_or(time::Date::MAX);
        if pe >= te {
            return true;
        }
        if pe >= cursor {
            cursor = pe.next_day().unwrap_or(pe);
        }
    }
    false
}

fn matches_text(filter: &Option<String>, value: &Option<String>) -> bool {
    match filter {
        None => true,
        Some(f) => value.as_deref().is_some_and(|v| v.to_lowercase().contains(&f.to_lowercase())),
    }
}

fn eval(c: &Criterion, facts: &FactSet) -> OcResult<Eval> {
    Ok(match c {
        Criterion::All { of } => {
            let parts = of.iter().map(|c| eval(c, facts)).collect::<OcResult<Vec<_>>>()?;
            let outcome = if parts.iter().any(|p| p.outcome == Tri::No) {
                Tri::No
            } else if parts.iter().all(|p| p.outcome == Tri::Yes) {
                Tri::Yes
            } else {
                Tri::Unknown
            };
            combine(outcome, parts)
        }
        Criterion::Any { of } => {
            let parts = of.iter().map(|c| eval(c, facts)).collect::<OcResult<Vec<_>>>()?;
            let outcome = if parts.iter().any(|p| p.outcome == Tri::Yes) {
                Tri::Yes
            } else if parts.iter().all(|p| p.outcome == Tri::No) {
                Tri::No
            } else {
                Tri::Unknown
            };
            combine(outcome, parts)
        }
        Criterion::Not { of } => {
            let inner = eval(of, facts)?;
            Eval {
                outcome: inner.outcome.not(),
                explanation: inner.explanation.into_iter().map(|e| format!("excluded if: {e}")).collect(),
                facts: inner.facts,
                missing: inner.missing,
            }
        }
        Criterion::ResidedIn { jurisdictions, during } => {
            let want: Vec<String> = jurisdictions.iter().map(|j| j.to_uppercase()).collect();
            let target = during.as_ref().map(Interval::span).transpose()?.unwrap_or(Span { start: None, end: None });
            let place = want.join("/");
            let window = describe_interval(during);
            let mut hits = Vec::new();
            let mut all_periods = Vec::new();
            for r in &facts.residences {
                let span = spans_of(&r.start, &r.end)?;
                all_periods.push(span);
                if want.contains(&r.jurisdiction.to_uppercase()) && span.overlaps(&target) {
                    hits.push(r.id.clone());
                }
            }
            if !hits.is_empty() {
                Eval::leaf(Tri::Yes, format!("you lived in {place}{window}"), hits, None)
            } else if covers(target, all_periods) {
                Eval::leaf(
                    Tri::No,
                    format!("your residence history covers{window} and does not include {place}"),
                    facts.residences.iter().map(|r| r.id.clone()).collect(),
                    None,
                )
            } else {
                Eval::leaf(
                    Tri::Unknown,
                    format!("no residence record shows {place}{window}"),
                    vec![],
                    Some(format!("Where did you live{window}? (needed: {place})")),
                )
            }
        }
        Criterion::ConsumerFact {
            fact_type,
            merchant,
            product,
            during,
        } => {
            let target = during.as_ref().map(Interval::span).transpose()?.unwrap_or(Span { start: None, end: None });
            let window = describe_interval(during);
            let what = [Some(fact_type.clone()), merchant.clone(), product.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" / ");
            let mut yes = Vec::new();
            let mut no = Vec::new();
            for f in &facts.consumer_facts {
                if !f.fact_type.eq_ignore_ascii_case(fact_type) || !matches_text(merchant, &f.merchant) || !matches_text(product, &f.product) {
                    continue;
                }
                let span = spans_of(&f.start, &f.end)?;
                if f.positive && span.overlaps(&target) {
                    yes.push(f.id.clone());
                } else if !f.positive && (span.start.is_none() && span.end.is_none() || span.overlaps(&target)) {
                    no.push(f.id.clone());
                }
            }
            match (yes.is_empty(), no.is_empty()) {
                (false, true) => Eval::leaf(Tri::Yes, format!("you recorded {what}{window}"), yes, None),
                (true, false) => Eval::leaf(Tri::No, format!("you stated you had no {what}{window}"), no, None),
                (false, false) => {
                    let mut ids = yes;
                    ids.extend(no);
                    Eval::leaf(
                        Tri::Unknown,
                        format!("your records about {what} contradict each other"),
                        ids,
                        Some(format!("Resolve the conflicting records about {what}")),
                    )
                }
                (true, true) => Eval::leaf(
                    Tri::Unknown,
                    format!("no record of {what}{window}"),
                    vec![],
                    Some(format!("Did you have {what}{window}?")),
                ),
            }
        }
        Criterion::Attestation { key, question } => match facts.attestations.get(key) {
            Some(true) => Eval::leaf(Tri::Yes, format!("you answered yes: {question}"), vec![format!("attestation:{key}")], None),
            Some(false) => Eval::leaf(Tri::No, format!("you answered no: {question}"), vec![format!("attestation:{key}")], None),
            None => Eval::leaf(Tri::Unknown, format!("not answered: {question}"), vec![], Some(question.clone())),
        },
    })
}

fn combine(outcome: Tri, parts: Vec<Eval>) -> Eval {
    let mut e = Eval {
        outcome,
        explanation: vec![],
        facts: vec![],
        missing: vec![],
    };
    for p in parts {
        e.explanation.extend(p.explanation);
        e.facts.extend(p.facts);
        e.missing.extend(p.missing);
    }
    e
}

fn describe_interval(i: &Option<Interval>) -> String {
    match i {
        None => String::new(),
        Some(Interval { start: Some(s), end: Some(e) }) => format!(" between {s} and {e}"),
        Some(Interval { start: Some(s), end: None }) => format!(" on or after {s}"),
        Some(Interval { start: None, end: Some(e) }) => format!(" on or before {e}"),
        Some(Interval { start: None, end: None }) => String::new(),
    }
}

/// Evaluate a verified rule set against one person's confirmed facts.
pub fn evaluate(rules: Option<&RuleSet>, facts: &FactSet) -> OcResult<Assessment> {
    if facts.not_in_class {
        return Ok(Assessment {
            state: MatchState::NotApplicable,
            summary: "You confirmed you are not part of this class.".into(),
            reasons: vec![],
            missing_facts: vec![],
            disclaimer: DISCLAIMER.into(),
        });
    }
    let Some(rules) = rules else {
        return Ok(Assessment {
            state: MatchState::ManualReview,
            summary: "No structured eligibility rules have been verified for this opportunity yet. Read the official terms.".into(),
            reasons: vec![],
            missing_facts: vec![],
            disclaimer: DISCLAIMER.into(),
        });
    };
    let mut reasons = Vec::new();
    let mut missing = Vec::new();
    let mut any_required_no = false;
    let mut any_required_unknown = false;
    let mut any_ambiguous = false;
    for r in &rules.rules {
        let e = eval(&r.criterion, facts)?;
        if r.required {
            any_required_no |= e.outcome == Tri::No;
            any_required_unknown |= e.outcome == Tri::Unknown;
        }
        any_ambiguous |= r.ambiguous;
        if e.outcome == Tri::Unknown {
            for m in &e.missing {
                if !missing.contains(m) {
                    missing.push(m.clone());
                }
            }
        }
        reasons.push(RuleReason {
            rule_id: r.id.clone(),
            label: r.label.clone(),
            required: r.required,
            outcome: e.outcome,
            explanation: e.explanation.join("; "),
            source_excerpt: r.source_excerpt.clone(),
            source_locator: r.source_locator.clone(),
            supporting_fact_ids: e.facts,
        });
    }
    let (state, summary) = if any_required_no {
        (MatchState::Unlikely, "At least one required condition conflicts with your records.")
    } else if any_ambiguous {
        (
            MatchState::ManualReview,
            "The official terms include conditions that need a careful human reading.",
        )
    } else if any_required_unknown {
        (
            MatchState::NeedsMoreInformation,
            "Some required conditions cannot be checked from your records yet.",
        )
    } else if reasons.iter().filter(|r| r.required).all(|r| r.outcome == Tri::Yes) {
        (
            MatchState::PotentialMatch,
            "Your records appear to support every required condition. Eligibility still requires review.",
        )
    } else {
        (MatchState::ManualReview, "The result could not be classified automatically.")
    };
    Ok(Assessment {
        state,
        summary: summary.into(),
        reasons,
        missing_facts: missing,
        disclaimer: DISCLAIMER.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(s: &str, e: &str) -> Option<Interval> {
        Some(Interval {
            start: Some(s.into()),
            end: Some(e.into()),
        })
    }

    fn rules() -> RuleSet {
        RuleSet {
            one_claim_per: Some("person".into()),
            rules: vec![
                Rule {
                    id: "residence".into(),
                    label: "California resident during class period".into(),
                    required: true,
                    ambiguous: false,
                    criterion: Criterion::ResidedIn {
                        jurisdictions: vec!["CA".into()],
                        during: interval("2019-01-01", "2021-12-31"),
                    },
                    source_excerpt: "All persons residing in California between January 1, 2019 and December 31, 2021".into(),
                    source_locator: "https://example.test/notice#class".into(),
                },
                Rule {
                    id: "purchase".into(),
                    label: "Bought the product".into(),
                    required: true,
                    ambiguous: false,
                    criterion: Criterion::ConsumerFact {
                        fact_type: "purchase".into(),
                        merchant: Some("Acme".into()),
                        product: None,
                        during: interval("2019-01-01", "2021-12-31"),
                    },
                    source_excerpt: "who purchased an Acme widget".into(),
                    source_locator: "https://example.test/notice#class".into(),
                },
                Rule {
                    id: "not_employee".into(),
                    label: "Not an Acme employee".into(),
                    required: true,
                    ambiguous: false,
                    criterion: Criterion::Not {
                        of: Box::new(Criterion::Attestation {
                            key: "acme_employee".into(),
                            question: "Were you an Acme employee?".into(),
                        }),
                    },
                    source_excerpt: "Excluded are employees of Acme".into(),
                    source_locator: "https://example.test/notice#exclusions".into(),
                },
            ],
        }
    }

    fn base_facts() -> FactSet {
        let mut f = FactSet::default();
        f.residences.push(ResidenceFact {
            id: "r1".into(),
            jurisdiction: "CA".into(),
            start: Some("2018-05-01".into()),
            end: Some("2020-06-30".into()),
        });
        f.consumer_facts.push(ConsumerFactRecord {
            id: "c1".into(),
            fact_type: "purchase".into(),
            merchant: Some("Acme Corp".into()),
            product: Some("widget".into()),
            start: Some("2020-02-02".into()),
            end: Some("2020-02-02".into()),
            positive: true,
        });
        f
    }

    #[test]
    fn missing_attestation_needs_more_information() {
        let a = evaluate(Some(&rules()), &base_facts()).unwrap();
        assert_eq!(a.state, MatchState::NeedsMoreInformation);
        assert_eq!(a.missing_facts, vec!["Were you an Acme employee?".to_string()]);
    }

    #[test]
    fn all_supported_is_potential_match_never_eligible() {
        let mut f = base_facts();
        f.attestations.insert("acme_employee".into(), false);
        let a = evaluate(Some(&rules()), &f).unwrap();
        assert_eq!(a.state, MatchState::PotentialMatch);
        assert!(a.disclaimer.contains("Only the official administrator"));
        assert!(a.reasons.iter().all(|r| !r.source_excerpt.is_empty()));
    }

    #[test]
    fn exclusion_makes_unlikely() {
        let mut f = base_facts();
        f.attestations.insert("acme_employee".into(), true);
        assert_eq!(evaluate(Some(&rules()), &f).unwrap().state, MatchState::Unlikely);
    }

    #[test]
    fn complete_residence_history_elsewhere_is_no() {
        let mut f = base_facts();
        f.residences = vec![ResidenceFact {
            id: "r2".into(),
            jurisdiction: "NY".into(),
            start: Some("2015-01-01".into()),
            end: None,
        }];
        f.attestations.insert("acme_employee".into(), false);
        let a = evaluate(Some(&rules()), &f).unwrap();
        assert_eq!(a.state, MatchState::Unlikely);
        assert_eq!(a.reasons[0].outcome, Tri::No);
    }

    #[test]
    fn partial_residence_history_elsewhere_is_unknown() {
        let mut f = base_facts();
        f.residences = vec![ResidenceFact {
            id: "r2".into(),
            jurisdiction: "NY".into(),
            start: Some("2020-01-01".into()),
            end: None,
        }];
        let a = evaluate(Some(&rules()), &f).unwrap();
        assert_eq!(a.reasons[0].outcome, Tri::Unknown);
    }

    #[test]
    fn outside_interval_does_not_count() {
        let mut f = base_facts();
        f.consumer_facts[0].start = Some("2023-01-01".into());
        f.consumer_facts[0].end = Some("2023-01-01".into());
        let a = evaluate(Some(&rules()), &f).unwrap();
        assert_eq!(a.reasons[1].outcome, Tri::Unknown);
    }

    #[test]
    fn contradictory_facts_are_unknown() {
        let mut f = base_facts();
        f.consumer_facts.push(ConsumerFactRecord {
            id: "c2".into(),
            fact_type: "purchase".into(),
            merchant: Some("Acme".into()),
            product: None,
            start: None,
            end: None,
            positive: false,
        });
        let a = evaluate(Some(&rules()), &f).unwrap();
        assert_eq!(a.reasons[1].outcome, Tri::Unknown);
        assert!(a.missing_facts.iter().any(|m| m.contains("conflicting")));
    }

    #[test]
    fn ambiguous_rule_forces_manual_review() {
        let mut r = rules();
        r.rules[0].ambiguous = true;
        let mut f = base_facts();
        f.attestations.insert("acme_employee".into(), false);
        assert_eq!(evaluate(Some(&r), &f).unwrap().state, MatchState::ManualReview);
    }

    #[test]
    fn no_rules_is_manual_review_and_not_in_class_is_not_applicable() {
        assert_eq!(evaluate(None, &base_facts()).unwrap().state, MatchState::ManualReview);
        let f = FactSet {
            not_in_class: true,
            ..Default::default()
        };
        assert_eq!(evaluate(Some(&rules()), &f).unwrap().state, MatchState::NotApplicable);
    }

    #[test]
    fn rule_sets_must_cite_sources() {
        let mut r = rules();
        r.rules[0].source_excerpt = " ".into();
        assert!(r.validate().is_err());
        assert!(rules().validate().is_ok());
    }
}
