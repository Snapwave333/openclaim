//! Stanford Securities Class Action Clearinghouse: the filings list (recent
//! securities class actions) and case pages (status, filing date, summary).
//! Authoritative and free; cases are leads for investors, not open claim forms.

use super::*;
use scraper::{Html, Selector};

pub const INDEX_URL: &str = "https://securities.stanford.edu/filings.html";

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn us_date(s: &str) -> Option<Date> {
    let parts: Vec<&str> = s.trim().split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    Date::from_calendar_date(
        parts[2].parse().ok()?,
        time::Month::try_from(parts[0].parse::<u8>().ok()?).ok()?,
        parts[1].parse().ok()?,
    )
    .ok()
}

fn blank(row: &ParsedOpportunity, title: String, key: String, url: String) -> ParsedOpportunity {
    ParsedOpportunity {
        external_key: key,
        title,
        official_url: url,
        ..row.clone()
    }
}

fn template() -> ParsedOpportunity {
    ParsedOpportunity {
        external_key: String::new(),
        title: String::new(),
        opp_type: OpportunityType::SecuritiesClassAction,
        official_url: String::new(),
        claim_url: None,
        administrator: None,
        administrator_phone: None,
        case_number: None,
        status: OpportunityStatus::Unknown,
        claim_deadline: None,
        class_period_start: None,
        class_period_end: None,
        jurisdictions: vec!["US".into()],
        summary: None,
        benefit: Benefit {
            amount_known: false,
            description: None,
        },
        proof_required: ProofRequirement::Unknown,
        claim_method: "unknown".into(),
        source_updated_on: None,
        terms_confirmed: false,
        evidence: vec![],
    }
}

pub fn parse_index(html: &str, base_url: &str) -> Vec<ParsedOpportunity> {
    let doc = Html::parse_document(html);
    let t = template();
    let mut out = Vec::new();
    for tr in doc.select(&sel("tr.table-link")) {
        let Some(onclick) = tr.value().attr("onclick") else { continue };
        let Some(rel) = onclick.split('\'').nth(1) else { continue };
        let Some(url) = absolute(base_url, rel) else { continue };
        let cells: Vec<String> = tr.select(&sel("td")).map(element_text).collect();
        if cells.len() < 3 || cells[0].is_empty() {
            continue;
        }
        let mut r = blank(&t, format!("{} securities class action", cells[0]), rel.to_string(), url.clone());
        r.source_updated_on = us_date(&cells[1]).map(format_date);
        r.summary = Some(format!(
            "Securities class action filed {} in {}{}.",
            cells[1],
            cells[2],
            cells.get(4).filter(|t| !t.is_empty()).map(|t| format!(" (ticker {t})")).unwrap_or_default()
        ));
        r.status = OpportunityStatus::Upcoming;
        r.evidence = vec![EvidenceSpan {
            locator: format!("{INDEX_URL}#{rel}"),
            assertion_type: "title".into(),
            excerpt: cells[..3.min(cells.len())].join(" | "),
        }];
        out.push(r);
    }
    out
}

pub fn parse_detail(html: &str, page_url: &str) -> Option<ParsedOpportunity> {
    let doc = Html::parse_document(html);
    let summary_section = doc.select(&sel("section#summary")).next()?;
    let text = element_text(summary_section);
    let title = summary_section.select(&sel("h4")).next().map(element_text).filter(|t| !t.is_empty())?;
    static STATUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Case Status:\s*(ONGOING|SETTLED|DISMISSED|[A-Z ]{4,20}?)\s").unwrap());
    static FILED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Filing Date:\s*([A-Za-z]+ \d{1,2}, \d{4})").unwrap());
    let status_word = STATUS.captures(&text).map(|c| c[1].trim().to_uppercase()).unwrap_or_default();
    let status = match status_word.as_str() {
        "SETTLED" => OpportunityStatus::AwaitingAdministrator,
        "DISMISSED" => OpportunityStatus::Closed,
        "ONGOING" => OpportunityStatus::Upcoming,
        _ => OpportunityStatus::Unknown,
    };
    let filed = FILED.captures(&text).and_then(|c| prose_to_date(&c[1])).map(format_date);
    let paragraphs: Vec<String> = summary_section
        .select(&sel("p"))
        .map(element_text)
        .filter(|p| !p.is_empty() && !p.starts_with("Case Status"))
        .collect();
    // The narrative follows the status/filing-date lines and is not wrapped in <p>.
    let summary = FILED
        .find(&text)
        .map(|m| collapse_ws(&text[m.end()..]))
        .filter(|t| t.len() > 40)
        .map(|t| t.chars().take(1200).collect::<String>())
        .or_else(|| paragraphs.iter().max_by_key(|p| p.len()).cloned());
    let period = find_class_period(&text);
    let rel = url::Url::parse(page_url)
        .map(|u| format!("{}{}", u.path().trim_start_matches('/'), u.query().map(|q| format!("?{q}")).unwrap_or_default()))
        .unwrap_or(page_url.into());
    let mut r = blank(&template(), title.clone(), rel, page_url.to_string());
    r.status = status;
    r.source_updated_on = filed;
    r.summary = summary.clone();
    r.class_period_start = period.as_ref().map(|p| p.0.clone());
    r.class_period_end = period.as_ref().map(|p| p.1.clone());
    r.terms_confirmed = true;
    r.evidence = vec![
        EvidenceSpan {
            locator: format!("{page_url}#summary h4"),
            assertion_type: "title".into(),
            excerpt: title,
        },
        EvidenceSpan {
            locator: format!("{page_url}#summary"),
            assertion_type: "status".into(),
            excerpt: format!("Case Status: {status_word}"),
        },
    ];
    if let Some(s) = summary {
        r.evidence.push(EvidenceSpan {
            locator: format!("{page_url}#summary p"),
            assertion_type: "summary".into(),
            excerpt: s,
        });
    }
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = include_str!("../../../../test-fixtures/html/scac_filings.html");
    const CASE: &str = include_str!("../../../../test-fixtures/html/scac_case_108640.html");

    #[test]
    fn filings_list_parses() {
        let rows = parse_index(INDEX, INDEX_URL);
        assert_eq!(rows.len(), 12);
        let lm = rows.iter().find(|r| r.title.starts_with("Lockheed Martin")).unwrap();
        assert_eq!(lm.official_url, "https://securities.stanford.edu/filings-case.html?id=108640");
        assert_eq!(lm.external_key, "filings-case.html?id=108640");
        assert_eq!(lm.source_updated_on.as_deref(), Some("2025-07-28"));
        assert_eq!(lm.opp_type, OpportunityType::SecuritiesClassAction);
    }

    #[test]
    fn case_page_parses_status() {
        let d = parse_detail(CASE, "https://securities.stanford.edu/filings-case.html?id=108640").unwrap();
        assert!(d.title.starts_with("Lockheed Martin"));
        assert_eq!(d.status, OpportunityStatus::Upcoming);
        assert_eq!(d.source_updated_on.as_deref(), Some("2025-07-28"));
        assert!(d.summary.as_deref().unwrap().contains("Complaint"));
        assert!(d.terms_confirmed);
        assert!(parse_detail("<html><body>nope</body></html>", INDEX_URL).is_none());
    }
}
