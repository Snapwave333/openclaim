//! Executes one connector refresh: fetch (index, then a bounded, rotating set
//! of detail pages), parse, and return everything with provenance. Persisting
//! is the daemon's job.

use crate::fetch::{FetchPolicy, Fetcher, default_user_agent};
use crate::manifest::{AuthRequirement, ConnectorKind, ConnectorManifest};
use crate::parse::{self, ParsedOpportunity, ParsedProgram, ParsedTip};
use openclaim_domain::model::FetchFailure;
use openclaim_security::digest::sha256_hex;
use std::time::Duration;
use time::Date;

#[derive(Debug, Clone)]
pub struct RunInput {
    pub manifest: ConnectorManifest,
    pub prior_etag: Option<String>,
    pub prior_last_modified: Option<String>,
    pub offline: bool,
    /// Contact string for sites whose fair-access rules require it.
    pub contact: Option<String>,
    pub today: Date,
    /// Detail pages (external keys) that are due for a fetch, in priority order.
    pub detail_due: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FetchedDoc {
    pub url: String,
    pub kind: String,
    pub status: u16,
    pub content_hash: String,
    pub body: String,
}

#[derive(Debug, Default)]
pub struct RunOutcome {
    pub docs: Vec<FetchedDoc>,
    pub opportunities: Vec<ParsedOpportunity>,
    pub programs: Vec<ParsedProgram>,
    pub tips: Vec<ParsedTip>,
    pub failure: Option<(FetchFailure, String, Option<u64>)>,
    pub requests: u32,
    pub not_modified: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub detail_fetched: Vec<String>,
    pub notes: Vec<String>,
}

pub fn fetch_policy(m: &ConnectorManifest, offline: bool, contact: Option<&str>) -> FetchPolicy {
    FetchPolicy {
        user_agent: default_user_agent(contact),
        allowed_hosts: m.allowed_hosts.clone(),
        min_interval: Duration::from_millis(m.rate_limit.min_interval_ms),
        max_body: crate::fetch::DEFAULT_MAX_BODY,
        timeout: Duration::from_secs(45),
        offline,
    }
}

/// Parses any page of a source, fetched or user-saved. Used by refresh and snapshot import.
pub type Parsed = (Vec<ParsedOpportunity>, Vec<ParsedProgram>, Vec<ParsedTip>);

pub fn parse_page(kind: ConnectorKind, url: &str, html: &str, today: Date) -> Result<Parsed, String> {
    let path = url::Url::parse(url).map(|u| u.path().trim_end_matches('/').to_string()).unwrap_or_default();
    match kind {
        ConnectorKind::ClassActionOrg => {
            let rows = parse::classaction_org::parse_index(html, url, today);
            if rows.is_empty() {
                return Err("no settlement cards found: the classaction.org layout may have changed".into());
            }
            Ok((rows, vec![], vec![]))
        }
        ConnectorKind::StanfordScac => {
            if path.ends_with("/filings.html") {
                let rows = parse::scac::parse_index(html, url);
                if rows.is_empty() {
                    return Err("no filings found: the SCAC layout may have changed".into());
                }
                Ok((rows, vec![], vec![]))
            } else {
                parse::scac::parse_detail(html, url)
                    .map(|o| (vec![o], vec![], vec![]))
                    .ok_or_else(|| "no case summary found on this page".into())
            }
        }
        ConnectorKind::Feed => {
            let tips = parse::feed::parse_feed(html.as_bytes());
            if tips.is_empty() {
                return Err("no feed entries parsed".into());
            }
            Ok((vec![], vec![], tips))
        }
    }
}

/// Fetches one official settlement page for verification. The allowlist is the page's own host;
/// every other defense (https only, SSRF-safe DNS, no cross-host redirects, bounded body) still applies.
pub async fn fetch_official_page(url: &str, offline: bool, contact: Option<&str>) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .ok_or_else(|| crate::fetch::FetchError {
            class: FetchFailure::PolicyBlocked,
            message: "official URL has no host".into(),
            retry_after_secs: None,
        })?;
    let fetcher = Fetcher::new(FetchPolicy {
        user_agent: default_user_agent(contact),
        allowed_hosts: vec![host],
        min_interval: Duration::from_millis(2000),
        max_body: crate::fetch::DEFAULT_MAX_BODY,
        timeout: Duration::from_secs(45),
        offline,
    })
    .map_err(|e| crate::fetch::FetchError {
        class: FetchFailure::NetworkError,
        message: e,
        retry_after_secs: None,
    })?;
    fetcher.get(url, None).await
}

pub async fn run(input: RunInput) -> RunOutcome {
    let m = &input.manifest;
    let mut out = RunOutcome::default();
    if m.auth_requirement == AuthRequirement::DeclaredContactUserAgent && input.contact.as_deref().is_none_or(|c| c.trim().is_empty()) {
        out.failure = Some((
            FetchFailure::PolicyBlocked,
            "this source requires a declared contact in the User-Agent; set `network.contact` first".into(),
            None,
        ));
        return out;
    }
    if !m.automation_allowed {
        out.failure = Some((
            FetchFailure::PolicyBlocked,
            "automated access is not permitted for this source; import a saved page instead".into(),
            None,
        ));
        return out;
    }
    let fetcher = match Fetcher::new(fetch_policy(m, input.offline, input.contact.as_deref())) {
        Ok(f) => f,
        Err(e) => {
            out.failure = Some((FetchFailure::NetworkError, e, None));
            return out;
        }
    };
    let index = match fetcher.get(&m.url, Some((input.prior_etag.clone(), input.prior_last_modified.clone()))).await {
        Ok(f) => f,
        Err(e) => {
            out.requests = *fetcher.requests_made.lock().await;
            out.failure = Some((e.class, e.message, e.retry_after_secs));
            return out;
        }
    };
    out.etag = index.etag.clone();
    out.last_modified = index.last_modified.clone();
    if index.not_modified {
        out.not_modified = true;
        out.requests = *fetcher.requests_made.lock().await;
    } else {
        let body = String::from_utf8_lossy(&index.body).into_owned();
        out.docs.push(FetchedDoc {
            url: index.url.clone(),
            kind: "index".into(),
            status: index.status,
            content_hash: sha256_hex(&index.body),
            body: body.clone(),
        });
        match parse_page(m.connector, &m.url, &body, input.today) {
            Ok((opps, programs, tips)) => {
                out.opportunities = opps;
                out.programs = programs;
                out.tips = tips;
            }
            Err(e) => {
                out.requests = *fetcher.requests_made.lock().await;
                let class = if e.contains("anti-automation") || e.contains("rate-limit") {
                    FetchFailure::AntiBotBlock
                } else {
                    FetchFailure::ParseError
                };
                out.failure = Some((class, e, None));
                return out;
            }
        }
    }
    if m.follow_detail_pages && m.max_detail_pages > 0 {
        let budget = (m.rate_limit.max_requests_per_run.saturating_sub(*fetcher.requests_made.lock().await)).min(m.max_detail_pages) as usize;
        let index_by_key: std::collections::HashMap<String, String> =
            out.opportunities.iter().map(|o| (o.external_key.clone(), o.official_url.clone())).collect();
        // First run: nothing is "due" yet, so take the freshly parsed index in order.
        let due: Vec<String> = if input.detail_due.is_empty() {
            out.opportunities.iter().map(|o| o.external_key.clone()).collect()
        } else {
            input.detail_due.clone()
        };
        for key in due.iter().take(budget) {
            let Some(url) = index_by_key.get(key).cloned().or_else(|| known_url(m, key)) else {
                continue;
            };
            match fetcher.get(&url, None).await {
                Ok(f) => {
                    let body = String::from_utf8_lossy(&f.body).into_owned();
                    out.docs.push(FetchedDoc {
                        url: f.url.clone(),
                        kind: "detail".into(),
                        status: f.status,
                        content_hash: sha256_hex(&f.body),
                        body: body.clone(),
                    });
                    match parse_page(m.connector, &url, &body, input.today) {
                        Ok((mut opps, _, _)) if !opps.is_empty() => {
                            let detailed = opps.remove(0);
                            if let Some(existing) = out.opportunities.iter_mut().find(|o| o.external_key == *key) {
                                merge_detail(existing, detailed);
                            } else {
                                out.opportunities.push(detailed);
                            }
                            out.detail_fetched.push(key.clone());
                        }
                        Ok(_) => out.notes.push(format!("{url}: no record parsed")),
                        Err(e) => out.notes.push(format!("{url}: {e}")),
                    }
                }
                Err(e) => {
                    out.notes.push(format!("{url}: {} ({})", e.message, e.class));
                    if matches!(e.class, FetchFailure::AntiBotBlock | FetchFailure::RateLimited | FetchFailure::RobotsDisallowed) {
                        out.failure = Some((e.class, e.message, e.retry_after_secs));
                        break;
                    }
                }
            }
        }
    }
    out.requests = *fetcher.requests_made.lock().await;
    out
}

fn known_url(m: &ConnectorManifest, external_key: &str) -> Option<String> {
    let base = url::Url::parse(&m.url).ok()?;
    base.join(external_key).ok().map(|u| u.to_string())
}

/// Detail pages carry the terms; index rows carry administrator contact and update dates.
pub fn merge_detail(index_row: &mut ParsedOpportunity, detail: ParsedOpportunity) {
    let admin = index_row.administrator.clone().or(detail.administrator.clone());
    let phone = index_row.administrator_phone.clone().or(detail.administrator_phone.clone());
    let updated = index_row.source_updated_on.clone().or(detail.source_updated_on.clone());
    let case_number = detail.case_number.clone().or(index_row.case_number.clone());
    let mut evidence = index_row.evidence.clone();
    evidence.extend(detail.evidence.clone());
    *index_row = ParsedOpportunity {
        administrator: admin,
        administrator_phone: phone,
        source_updated_on: updated,
        case_number,
        evidence,
        ..detail
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_page_routes_by_url() {
        let today = openclaim_domain::dates::parse_date("2026-10-19").unwrap();
        let (o, _, _) = parse_page(
            ConnectorKind::ClassActionOrg,
            "https://www.classaction.org/settlements",
            include_str!("../../../test-fixtures/html/classaction_org_settlements.html"),
            today,
        )
        .unwrap();
        assert_eq!(o.len(), 8);
        let (o, _, _) = parse_page(
            ConnectorKind::StanfordScac,
            "https://securities.stanford.edu/filings.html",
            include_str!("../../../test-fixtures/html/scac_filings.html"),
            today,
        )
        .unwrap();
        assert_eq!(o.len(), 12);
        let (o, _, _) = parse_page(
            ConnectorKind::StanfordScac,
            "https://securities.stanford.edu/filings-case.html?id=108640",
            include_str!("../../../test-fixtures/html/scac_case_108640.html"),
            today,
        )
        .unwrap();
        assert_eq!(o.len(), 1);
        assert!(
            parse_page(
                ConnectorKind::ClassActionOrg,
                "https://www.classaction.org/settlements",
                "<p>changed</p>",
                today
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn offline_run_reports_offline() {
        let m = crate::manifest::bundled().into_iter().find(|m| m.id == "classaction-org").unwrap();
        let out = run(RunInput {
            manifest: m,
            prior_etag: None,
            prior_last_modified: None,
            offline: true,
            contact: None,
            today: openclaim_domain::dates::parse_date("2026-10-19").unwrap(),
            detail_due: vec![],
        })
        .await;
        assert_eq!(out.failure.unwrap().0, FetchFailure::Offline);
    }
}
