//! FTC "Recent FTC Cases Resulting in Refunds" index and per-program pages.

use super::*;
use scraper::{Html, Selector};

pub const INDEX_URL: &str = "https://www.ftc.gov/enforcement/refunds";

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

/// Index rows: title + program page, month updated, phone + administrator.
pub fn parse_index(html: &str, base_url: &str) -> Vec<ParsedOpportunity> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    for row in doc.select(&sel("table.usa-table tbody tr")) {
        let Some(link) = row.select(&sel("td.views-field-title a")).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else { continue };
        let Some(url) = absolute(base_url, href) else { continue };
        let title = element_text(link);
        if title.is_empty() {
            continue;
        }
        let updated = row
            .select(&sel("td.views-field-field-date time"))
            .next()
            .and_then(|t| t.value().attr("datetime").map(|d| d[..10.min(d.len())].to_string()));
        let contact_cell = row.select(&sel("td.views-field-field-phone")).next();
        let (phone, administrator) = contact_cell
            .map(|c| {
                let parts: Vec<String> = c.text().map(collapse_ws).filter(|s| !s.is_empty()).collect();
                let phone = parts.iter().find(|p| p.chars().filter(|c| c.is_ascii_digit()).count() >= 10).cloned();
                let admin = parts.iter().find(|p| !p.chars().any(|c| c.is_ascii_digit())).cloned();
                (phone, admin)
            })
            .unwrap_or((None, None));
        let path = url::Url::parse(&url).map(|u| u.path().to_string()).unwrap_or(url.clone());
        out.push(ParsedOpportunity {
            external_key: path.clone(),
            title: title.clone(),
            opp_type: OpportunityType::GovernmentRefund,
            official_url: url.clone(),
            claim_url: None,
            administrator,
            administrator_phone: phone,
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
            source_updated_on: updated,
            terms_confirmed: false,
            evidence: vec![EvidenceSpan {
                locator: format!("{INDEX_URL}#table.usa-table a[href='{path}']"),
                assertion_type: "title".into(),
                excerpt: title,
            }],
        });
    }
    out
}

/// A program page: body paragraphs carry who gets money, how, and any claim deadline.
pub fn parse_detail(html: &str, page_url: &str, today: Date) -> Option<ParsedOpportunity> {
    let doc = Html::parse_document(html);
    let article = doc.select(&sel("article.node--type-refund")).next()?;
    let title = article
        .select(&sel("[data-a2a-title]"))
        .next()
        .and_then(|e| e.value().attr("data-a2a-title").map(collapse_ws))
        .or_else(|| doc.select(&sel("h1")).next().map(element_text))
        .or_else(|| {
            doc.select(&sel("title"))
                .next()
                .map(|t| element_text(t).split('|').next().unwrap_or("").trim().to_string())
        })
        .filter(|t| !t.is_empty())
        .or_else(|| article.value().attr("about").map(|a| a.rsplit('/').next().unwrap_or(a).replace('-', " ")))
        .unwrap_or_else(|| "FTC refund program".into());
    let body = article.select(&sel("div.field--name-body")).next()?;
    let mut paragraphs: Vec<String> = body
        .select(&sel("p"))
        .map(element_text)
        .filter(|p| !p.is_empty() && !p.to_lowercase().contains("impersonat") && !p.to_lowercase().contains("scammers are"))
        .collect();
    if paragraphs.is_empty() {
        paragraphs.push(element_text(body));
    }
    let text = paragraphs.join(" ");
    let updated = article
        .select(&sel("div.field--name-field-date time"))
        .next()
        .and_then(|t| t.value().attr("datetime").map(|d| d[..10.min(d.len())].to_string()));
    let mut evidence = vec![EvidenceSpan {
        locator: format!("{page_url}#h1"),
        assertion_type: "title".into(),
        excerpt: title.clone(),
    }];
    let summary = paragraphs.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    evidence.push(EvidenceSpan {
        locator: format!("{page_url}#div.field--name-body p:nth-of-type(1)"),
        assertion_type: "summary".into(),
        excerpt: summary.clone(),
    });

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
    let mut administrator = None;
    let mut phone = None;
    static ADMIN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)refund administrator,?\s+([A-Z][A-Za-z&.,' -]{2,60}?),?\s+at\s+(1-\d{3}-\d{3}-\d{4}|\(\d{3}\)\s?\d{3}-\d{4}|\d{3}-\d{3}-\d{4})")
            .unwrap()
    });
    if let Some(c) = ADMIN.captures(&text) {
        administrator = Some(collapse_ws(&c[1]).trim_end_matches(',').to_string());
        phone = Some(c[2].to_string());
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
        let link_text = element_text(a).to_lowercase();
        let looks_like_claim = link_text.contains("claim")
            || link_text.contains("apply")
            || link_text.contains("file")
            || host.contains("refund")
            || host.contains("settlement")
            || host.contains("claim");
        if !host.ends_with("ftc.gov") && !host.ends_with("usa.gov") && looks_like_claim {
            claim_url = Some(u.clone());
            evidence.push(EvidenceSpan {
                locator: format!("{page_url}#a[href='{href}']"),
                assertion_type: "claim_url".into(),
                excerpt: element_text(a),
            });
            break;
        }
    }
    let proof_required = if text.to_lowercase().contains("receipt") || text.to_lowercase().contains("proof of purchase") {
        ProofRequirement::Required
    } else if claim_method == "automatic" {
        ProofRequirement::None
    } else {
        ProofRequirement::Unknown
    };
    let path = url::Url::parse(page_url).map(|u| u.path().to_string()).unwrap_or(page_url.to_string());
    Some(ParsedOpportunity {
        external_key: path,
        title,
        opp_type: OpportunityType::GovernmentRefund,
        official_url: page_url.to_string(),
        claim_url,
        administrator,
        administrator_phone: phone,
        case_number: None,
        status,
        claim_deadline: deadline.map(|d| d.0),
        class_period_start: period.as_ref().map(|p| p.0.clone()),
        class_period_end: period.as_ref().map(|p| p.1.clone()),
        jurisdictions: vec!["US".into()],
        summary: Some(summary),
        benefit: find_benefit(&text),
        proof_required,
        claim_method,
        source_updated_on: updated,
        terms_confirmed: true,
        evidence,
    })
}

/// Is this HTML the FTC anti-automation interstitial rather than content?
pub fn is_block_page(html: &str) -> bool {
    let l = html.to_lowercase();
    l.contains("resembles an abusive automated request") || l.contains("apology_objects")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = include_str!("../../../../test-fixtures/html/ftc_refunds_index.html");
    const WALMART: &str = include_str!("../../../../test-fixtures/html/ftc_refund_walmart.html");

    #[test]
    fn index_rows_parse() {
        let rows = parse_index(INDEX, INDEX_URL);
        assert!(rows.len() >= 20, "got {}", rows.len());
        let w = rows.iter().find(|r| r.title == "Walmart Refunds").unwrap();
        assert_eq!(w.official_url, "https://www.ftc.gov/enforcement/refunds/walmart-refunds");
        assert_eq!(w.administrator.as_deref(), Some("Rust Consulting, Inc."));
        assert_eq!(w.administrator_phone.as_deref(), Some("1-833-472-1990"));
        assert_eq!(w.source_updated_on.as_deref(), Some("2026-10-15"));
        assert!(!w.terms_confirmed);
        let amazon = rows.iter().find(|r| r.title == "Amazon Refunds").unwrap();
        assert!(amazon.administrator.is_none());
    }

    #[test]
    fn detail_page_parses_terms() {
        let d = parse_detail(
            WALMART,
            "https://www.ftc.gov/enforcement/refunds/walmart-refunds",
            parse_date("2026-10-19").unwrap(),
        )
        .unwrap();
        assert_eq!(d.title, "Walmart Refunds");
        assert_eq!(d.status, OpportunityStatus::AutomaticDistribution);
        assert_eq!(d.claim_method, "automatic");
        assert_eq!(d.class_period_start.as_deref(), Some("2015-01-01"));
        assert_eq!(d.class_period_end.as_deref(), Some("2024-12-31"));
        assert_eq!(d.administrator.as_deref(), Some("Rust Consulting"));
        assert!(d.summary.as_deref().unwrap().contains("65,022 checks"));
        assert!(d.terms_confirmed);
        assert!(d.evidence.iter().any(|e| e.assertion_type == "class_period"));
        assert!(!d.benefit.amount_known);
        assert!(d.claim_url.is_none());
    }

    #[test]
    fn block_page_detection_and_bad_layout() {
        assert!(is_block_page(
            "<img src=\"/apology_objects/logo.jpg\"> The request resembles an abusive automated request."
        ));
        assert!(parse_index("<html><body><p>Layout changed</p></body></html>", INDEX_URL).is_empty());
        assert!(parse_detail("<html><body><p>Layout changed</p></body></html>", INDEX_URL, parse_date("2026-10-19").unwrap()).is_none());
    }
}
