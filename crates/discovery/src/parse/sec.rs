//! SEC "Distributions to Harmed Investors" index and case pages.

use super::*;
use scraper::{Html, Selector};

pub const INDEX_URL: &str = "https://www.sec.gov/enforcement-litigation/distributions-harmed-investors";

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn case_number_from_url(u: &str) -> Option<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:case-no|file-no|admin-proc-file-no)-([0-9a-z-]+?)(?:-[a-z]{2,6})*$").unwrap());
    let path = url::Url::parse(u).ok()?.path().to_string();
    let c = RE.captures(&path)?;
    Some(c[1].to_string())
}

pub fn parse_index(html: &str, base_url: &str) -> Vec<ParsedOpportunity> {
    let doc = Html::parse_document(html);
    let body = doc.select(&sel("div.field--name-body")).next();
    let Some(body) = body else { return Vec::new() };
    let mut best: Vec<ParsedOpportunity> = Vec::new();
    for ul in body.select(&sel("ul")) {
        let mut rows = Vec::new();
        for li in ul.children().filter_map(ElementRef::wrap).filter(|e| e.value().name() == "li") {
            let Some(a) = li.select(&sel("a[href]")).next() else { continue };
            let Some(href) = a.value().attr("href") else { continue };
            let Some(u) = absolute(base_url, href) else { continue };
            let title = element_text(a);
            // The list also links a notices page and the completed-distributions archive; those are not cases.
            if title.is_empty() || title.starts_with("Distributions in Commission") || title.starts_with("Archive of") {
                continue;
            }
            let path = url::Url::parse(&u)
                .map(|x| format!("{}{}", x.path(), x.fragment().map(|f| format!("#{f}")).unwrap_or_default()))
                .unwrap_or(u.clone());
            rows.push(ParsedOpportunity {
                external_key: path.clone(),
                title: title.clone(),
                opp_type: OpportunityType::InvestorDistribution,
                official_url: u.clone(),
                claim_url: None,
                administrator: None,
                administrator_phone: None,
                case_number: case_number_from_url(&u),
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
                evidence: vec![EvidenceSpan {
                    locator: format!("{INDEX_URL}#a[href='{path}']"),
                    assertion_type: "title".into(),
                    excerpt: title,
                }],
            });
        }
        if rows.len() > best.len() {
            best = rows;
        }
    }
    best
}

pub fn parse_detail(html: &str, page_url: &str, today: Date) -> Option<ParsedOpportunity> {
    let doc = Html::parse_document(html);
    let title = doc
        .select(&sel("h1.page-title__heading, h1"))
        .next()
        .map(element_text)
        .filter(|t| !t.is_empty())?;
    let body = doc.select(&sel("div.field--name-body")).next()?;
    let paragraphs: Vec<String> = body.select(&sel("p")).map(element_text).filter(|p| !p.is_empty()).collect();
    if paragraphs.is_empty() {
        return None;
    }
    let text = paragraphs.join(" ");
    let summary = paragraphs.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    let mut evidence = vec![
        EvidenceSpan {
            locator: format!("{page_url}#h1"),
            assertion_type: "title".into(),
            excerpt: title.clone(),
        },
        EvidenceSpan {
            locator: format!("{page_url}#div.field--name-body p:nth-of-type(1)"),
            assertion_type: "summary".into(),
            excerpt: summary.clone(),
        },
    ];
    let deadline = find_deadline(&text);
    if let Some((_, s)) = &deadline {
        evidence.push(EvidenceSpan {
            locator: format!("{page_url}#div.field--name-body"),
            assertion_type: "deadline".into(),
            excerpt: s.clone(),
        });
    }
    let (status, claim_method, status_excerpt) = infer_status(&text, deadline.as_ref().map(|d| d.0.as_str()), today);
    if !status_excerpt.is_empty() {
        evidence.push(EvidenceSpan {
            locator: format!("{page_url}#div.field--name-body"),
            assertion_type: "status".into(),
            excerpt: status_excerpt,
        });
    }
    let period = find_class_period(&text);
    if let Some((_, _, s)) = &period {
        evidence.push(EvidenceSpan {
            locator: format!("{page_url}#div.field--name-body"),
            assertion_type: "class_period".into(),
            excerpt: s.clone(),
        });
    }
    static ADMIN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)appointing\s+([A-Z][A-Za-z0-9&.,' -]{2,80}?),?\s+as\s+(?:the\s+)?(?:fund\s+administrator|distribution\s+agent)").unwrap()
    });
    let mut administrator = None;
    if let Some(c) = ADMIN.captures(&text) {
        administrator = Some(collapse_ws(&c[1]).trim_end_matches(',').to_string());
        evidence.push(EvidenceSpan {
            locator: format!("{page_url}#div.field--name-body"),
            assertion_type: "administrator".into(),
            excerpt: sentence_around(&text, c.get(0).unwrap().start(), c.get(0).unwrap().end()),
        });
    }
    let mut claim_url = None;
    for a in body.select(&sel("a[href]")) {
        let Some(href) = a.value().attr("href") else { continue };
        let Some(u) = absolute(page_url, href) else { continue };
        let host = url::Url::parse(&u).ok().and_then(|x| x.host_str().map(|h| h.to_string())).unwrap_or_default();
        let t = element_text(a).to_lowercase();
        if !host.ends_with("sec.gov")
            && (t.contains("claim") || t.contains("fund") || host.contains("claim") || host.contains("settlement") || host.contains("fairfund"))
        {
            claim_url = Some(u);
            evidence.push(EvidenceSpan {
                locator: format!("{page_url}#a[href='{href}']"),
                assertion_type: "claim_url".into(),
                excerpt: element_text(a),
            });
            break;
        }
    }
    let updated = doc
        .select(&sel("div.date-modified span.nowrap"))
        .next()
        .map(element_text)
        .and_then(|s| prose_to_date(&s).map(format_date));
    let path = url::Url::parse(page_url).map(|u| u.path().to_string()).unwrap_or(page_url.to_string());
    Some(ParsedOpportunity {
        external_key: path,
        title,
        opp_type: OpportunityType::InvestorDistribution,
        official_url: page_url.to_string(),
        claim_url,
        administrator,
        administrator_phone: None,
        case_number: case_number_from_url(page_url),
        status,
        claim_deadline: deadline.map(|d| d.0),
        class_period_start: period.as_ref().map(|p| p.0.clone()),
        class_period_end: period.as_ref().map(|p| p.1.clone()),
        jurisdictions: vec!["US".into()],
        summary: Some(summary),
        benefit: find_benefit(&text),
        proof_required: ProofRequirement::Unknown,
        claim_method,
        source_updated_on: updated,
        terms_confirmed: true,
        evidence,
    })
}

pub fn is_block_page(html: &str) -> bool {
    let l = html.to_lowercase();
    l.contains("request rate threshold exceeded") || l.contains("automated access to our sites must comply")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = include_str!("../../../../test-fixtures/html/sec_distributions_index.html");
    const ALLIANZ: &str = include_str!("../../../../test-fixtures/html/sec_case_allianz.html");

    #[test]
    fn index_lists_cases() {
        let rows = parse_index(INDEX, INDEX_URL);
        assert!(rows.len() > 300, "got {}", rows.len());
        let a = rows.iter().find(|r| r.title.starts_with("Allianz Global")).unwrap();
        assert_eq!(
            a.official_url,
            "https://www.sec.gov/enforcement-litigation/distributions-harmed-investors/matter-allianz-global-investors-us-llc-admin-proc-file-no-3-20855"
        );
        assert_eq!(a.case_number.as_deref(), Some("3-20855"));
        assert!(!a.terms_confirmed);
        assert!(rows.iter().all(|r| r.title != "Mark McKinnies"), "nested individuals must not become cases");
    }

    #[test]
    fn case_page_parses() {
        let d = parse_detail(
            ALLIANZ,
            "https://www.sec.gov/enforcement-litigation/distributions-harmed-investors/matter-allianz-global-investors-us-llc-admin-proc-file-no-3-20855",
            parse_date("2026-10-19").unwrap(),
        )
        .unwrap();
        assert!(d.title.contains("Allianz"));
        assert_eq!(d.status, OpportunityStatus::AwaitingAdministrator);
        assert_eq!(d.administrator.as_deref(), Some("Epiq Class Action & Claims Solutions, Inc."));
        assert_eq!(d.class_period_start.as_deref(), Some("2020-02-24"));
        assert_eq!(d.class_period_end.as_deref(), Some("2020-03-30"));
        assert!(d.benefit.description.as_deref().unwrap().contains("$675,000,000.00"));
        assert!(d.terms_confirmed);
    }

    #[test]
    fn rate_limit_page_detected() {
        assert!(is_block_page("<title>SEC.gov | Request Rate Threshold Exceeded</title>"));
        assert!(parse_index("<html><body>nothing</body></html>", INDEX_URL).is_empty());
    }
}
