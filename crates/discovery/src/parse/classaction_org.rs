//! classaction.org "Open Settlements" listing: one card per settlement with the
//! official administrator site, claim deadline, payout note, proof requirement
//! and a one-sentence class description. A third-party listing (Tier B): records
//! stay `verification_pending` until the official site is checked.

use super::*;
use scraper::{Html, Selector};

pub const INDEX_URL: &str = "https://www.classaction.org/settlements";

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

/// "11/27/26" → 2026-11-27.
fn short_date(s: &str) -> Option<Date> {
    let parts: Vec<&str> = s.trim().split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let (m, d, y) = (parts[0].parse::<u8>().ok()?, parts[1].parse::<u8>().ok()?, parts[2].parse::<i32>().ok()?);
    let year = if y < 100 { 2000 + y } else { y };
    Date::from_calendar_date(year, time::Month::try_from(m).ok()?, d).ok()
}

pub fn parse_index(html: &str, base_url: &str, today: Date) -> Vec<ParsedOpportunity> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    for card in doc.select(&sel("div.settlement-card")) {
        let Some(slug) = card.value().attr("id").filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(link) = card.select(&sel("a.js-settlement-link[href]")).next() else {
            continue;
        };
        // Regular cards carry data-name on the card; the featured card carries it on the link.
        let name = card
            .value()
            .attr("data-name")
            .or_else(|| link.value().attr("data-name"))
            .map(collapse_ws)
            .unwrap_or_default();
        let Some(official) = link.value().attr("href").and_then(|h| absolute(base_url, h)) else {
            continue;
        };
        if !openclaim_security::netpolicy::is_safe_display_link(&official) {
            continue;
        }
        let title = if name.is_empty() { element_text(link) } else { name.clone() };
        // The stats strip holds three label/value pairs: payout, deadline, proof required.
        let values: Vec<String> = card.select(&sel("span.db.f4, span.db.f5")).map(element_text).collect();
        let labels: Vec<String> = card.select(&sel("span.db.tw-gray")).map(|e| element_text(e).to_lowercase()).collect();
        let mut payout = None;
        let mut deadline_text = None;
        let mut proof_text = None;
        for (label, value) in labels.iter().zip(values.iter()) {
            if label.contains("payout") {
                payout = Some(value.clone());
            } else if label.contains("deadline") {
                deadline_text = Some(value.clone());
            } else if label.contains("proof") {
                proof_text = Some(value.clone());
            }
        }
        let summary = card.select(&sel("p")).map(element_text).find(|p| !p.is_empty());
        let deadline = deadline_text.as_deref().and_then(short_date);
        let locator = format!("{INDEX_URL}#{slug}");
        let mut evidence = vec![EvidenceSpan {
            locator: locator.clone(),
            assertion_type: "title".into(),
            excerpt: title.clone(),
        }];
        if let Some(s) = &summary {
            evidence.push(EvidenceSpan {
                locator: locator.clone(),
                assertion_type: "summary".into(),
                excerpt: s.clone(),
            });
        }
        if let Some(d) = &deadline_text {
            evidence.push(EvidenceSpan {
                locator: locator.clone(),
                assertion_type: "deadline".into(),
                excerpt: format!("Deadline: {d}"),
            });
        }
        evidence.push(EvidenceSpan {
            locator: locator.clone(),
            assertion_type: "claim_url".into(),
            excerpt: official.clone(),
        });
        let status = match deadline {
            Some(d) if d >= today => OpportunityStatus::Open,
            Some(_) => OpportunityStatus::PastDeadline,
            None => OpportunityStatus::Open,
        };
        let proof_required = match proof_text.as_deref().map(|p| p.to_lowercase()) {
            Some(p) if p.starts_with("yes") => ProofRequirement::Required,
            Some(p) if p.starts_with("no") => ProofRequirement::None,
            _ => ProofRequirement::Unknown,
        };
        let period = summary.as_deref().and_then(find_class_period);
        out.push(ParsedOpportunity {
            external_key: slug.to_string(),
            title,
            opp_type: OpportunityType::ClassAction,
            official_url: official.clone(),
            claim_url: Some(official),
            administrator: None,
            administrator_phone: None,
            case_number: None,
            status,
            claim_deadline: deadline.map(format_date),
            class_period_start: period.as_ref().map(|p| p.0.clone()),
            class_period_end: period.as_ref().map(|p| p.1.clone()),
            jurisdictions: vec!["US".into()],
            summary,
            benefit: Benefit {
                amount_known: false,
                description: payout.filter(|p| !p.is_empty()).map(|p| format!("Listed payout: {p} (not guaranteed)")),
            },
            proof_required,
            claim_method: "online_form".into(),
            source_updated_on: None,
            terms_confirmed: false,
            evidence,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../../../../test-fixtures/html/classaction_org_settlements.html");

    #[test]
    fn cards_parse_with_official_links_and_deadlines() {
        let rows = parse_index(PAGE, INDEX_URL, parse_date("2026-10-19").unwrap());
        assert_eq!(rows.len(), 8);
        let squatch = rows.iter().find(|r| r.external_key == "dr-squatch-all-natural-claims").unwrap();
        assert_eq!(squatch.official_url, "https://www.personalcareproductssettlement.com/");
        assert_eq!(squatch.claim_deadline.as_deref(), Some("2026-11-27"));
        assert_eq!(squatch.status, OpportunityStatus::Open);
        assert_eq!(squatch.proof_required, ProofRequirement::None);
        assert_eq!(squatch.class_period_start.as_deref(), Some("2018-11-01"));
        assert!(squatch.summary.as_deref().unwrap().contains("Dr. Squatch"));
        assert!(!squatch.terms_confirmed, "third-party listing is never verified by itself");
        let kaiser = rows.iter().find(|r| r.external_key == "kaiser-permanente-out-of-network-claims").unwrap();
        assert!(kaiser.claim_deadline.is_none(), "'Varies' is not a deadline");
        assert_eq!(kaiser.proof_required, ProofRequirement::Required);
        assert!(rows.iter().all(|r| r.evidence.iter().any(|e| e.assertion_type == "claim_url")));
    }

    #[test]
    fn changed_layout_yields_nothing() {
        assert!(parse_index("<html><body><p>new layout</p></body></html>", INDEX_URL, parse_date("2026-10-19").unwrap()).is_empty());
        assert_eq!(short_date("11/27/26"), Some(parse_date("2026-11-27").unwrap()));
        assert_eq!(short_date("Varies"), None);
    }
}
