//! Pure parsers. Input: HTML or feed text; output: normalized records with
//! evidence spans pointing back at the source text. No network here.

pub mod feed;
pub mod ftc;
pub mod naupa;
pub mod sec;

use openclaim_domain::dates::{format_date, month_from_name, parse_date};
use openclaim_domain::model::{OpportunityStatus, OpportunityType, ProofRequirement};
use regex::Regex;
use schemars::JsonSchema;
use scraper::ElementRef;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use time::Date;

pub const PARSER_VERSION: &str = "2026.10.1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct EvidenceSpan {
    /// Where in the source the assertion comes from (URL + CSS path or quote).
    pub locator: String,
    /// What the span supports: `title`, `status`, `deadline`, `class_period`, `administrator`, `claim_url`, `summary`.
    pub assertion_type: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Benefit {
    pub amount_known: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ParsedOpportunity {
    /// Stable key within the source (usually the page path).
    pub external_key: String,
    pub title: String,
    pub opp_type: OpportunityType,
    pub official_url: String,
    pub claim_url: Option<String>,
    pub administrator: Option<String>,
    pub administrator_phone: Option<String>,
    pub case_number: Option<String>,
    pub status: OpportunityStatus,
    pub claim_deadline: Option<String>,
    pub class_period_start: Option<String>,
    pub class_period_end: Option<String>,
    pub jurisdictions: Vec<String>,
    pub summary: Option<String>,
    pub benefit: Benefit,
    pub proof_required: ProofRequirement,
    /// `online_form`, `automatic`, `mail`, `unknown`.
    pub claim_method: String,
    pub source_updated_on: Option<String>,
    /// Detail pages carry the terms; index rows alone do not confirm class terms.
    pub terms_confirmed: bool,
    pub evidence: Vec<EvidenceSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ParsedProgram {
    pub jurisdiction: String,
    pub name: String,
    pub official_url: String,
    /// `us_state`, `us_territory`, `federal`, `multi_state`, `canada`, `other`.
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ParsedTip {
    pub url: String,
    pub title: String,
    pub published: Option<String>,
    pub summary: Option<String>,
}

pub fn element_text(el: ElementRef<'_>) -> String {
    let raw: String = el.text().collect::<Vec<_>>().join(" ");
    collapse_ws(&raw)
}

pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn absolute(base: &str, href: &str) -> Option<String> {
    let b = url::Url::parse(base).ok()?;
    let u = b.join(href.trim()).ok()?;
    if !matches!(u.scheme(), "https" | "http") {
        return None;
    }
    Some(u.to_string())
}

static PROSE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)\.?\s+(\d{1,2}),?\s+(\d{4})\b").unwrap()
});
static MONTH_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(January|February|March|April|May|June|July|August|September|October|November|December)\s+(\d{4})\b").unwrap());
static DEADLINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)(?:claim[s]?\s+(?:form[s]?\s+)?(?:must\s+be\s+)?(?:file[d]?|submit(?:ted)?|postmarked|received)|deadline(?:\s+to\s+file)?(?:\s+is)?|file\s+(?:a|your)\s+claim|submit\s+(?:a|your)\s+claim|apply\s+for\s+a\s+(?:refund|payment))[^.]{0,80}?\b(?:by|before|no\s+later\s+than|until|on\s+or\s+before|is)\s+((?:Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)\.?\s+\d{1,2},?\s+\d{4})").unwrap()
});
static CLASS_PERIOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbetween\s+((?:January|February|March|April|May|June|July|August|September|October|November|December)(?:\s+\d{1,2},?)?\s+\d{4})\s+(?:and|through|to)\s+((?:January|February|March|April|May|June|July|August|September|October|November|December)(?:\s+\d{1,2},?)?\s+\d{4})").unwrap()
});

pub fn prose_to_date(s: &str) -> Option<Date> {
    let c = PROSE_DATE.captures(s)?;
    let m = month_from_name(&c[1])?;
    let iso = format!("{}-{:02}-{:0>2}", &c[3], m, &c[2]);
    parse_date(&iso).ok()
}

/// "January 2015" → first day; with `end` the last day of that month.
pub fn month_year_to_date(s: &str, end: bool) -> Option<Date> {
    if let Some(d) = prose_to_date(s) {
        return Some(d);
    }
    let c = MONTH_YEAR.captures(s)?;
    let m = month_from_name(&c[1])?;
    let y: i32 = c[2].parse().ok()?;
    let month = time::Month::try_from(m).ok()?;
    let first = Date::from_calendar_date(y, month, 1).ok()?;
    if end {
        let next = if m == 12 {
            Date::from_calendar_date(y + 1, time::Month::January, 1)
        } else {
            Date::from_calendar_date(y, month.next(), 1)
        };
        next.ok()?.previous_day()
    } else {
        Some(first)
    }
}

/// Finds "file a claim by <date>"-style statements. Returns the date and the sentence.
pub fn find_deadline(text: &str) -> Option<(String, String)> {
    let c = DEADLINE.captures(text)?;
    let d = prose_to_date(&c[1])?;
    let whole = c.get(0)?;
    let sentence = sentence_around(text, whole.start(), whole.end());
    Some((format_date(d), sentence))
}

pub fn find_class_period(text: &str) -> Option<(String, String, String)> {
    let c = CLASS_PERIOD.captures(text)?;
    let s = month_year_to_date(&c[1], false)?;
    let e = month_year_to_date(&c[2], true)?;
    let whole = c.get(0)?;
    Some((format_date(s), format_date(e), sentence_around(text, whole.start(), whole.end())))
}

pub fn sentence_around(text: &str, start: usize, end: usize) -> String {
    let s = text[..start].rfind(". ").map(|i| i + 2).unwrap_or(0);
    let e = text[end..].find(". ").map(|i| end + i + 1).unwrap_or(text.len());
    collapse_ws(&text[s..e])
}

/// Deterministic claim status from official wording, with the sentence that supports it.
pub fn infer_status(text: &str, deadline: Option<&str>, today: Date) -> (OpportunityStatus, String, String) {
    let lower = text.to_lowercase();
    if let Some(d) = deadline
        && let Ok(dd) = parse_date(d)
    {
        return if dd >= today {
            (OpportunityStatus::Open, "online_form".into(), format!("claim deadline {d} is in the future"))
        } else {
            (OpportunityStatus::PastDeadline, "online_form".into(), format!("claim deadline {d} has passed"))
        };
    }
    const AUTOMATIC: &[&str] = &[
        "is sending",
        "are sending",
        "sent payments",
        "sending payments",
        "sending checks",
        "mailed checks",
        "checks totaling",
        "payments totaling",
        "paypal payments",
        "will receive a payment",
        "no action is required",
        "do not need to file",
        "distributed directly",
        "payments will be made",
        "sending more than",
        "sent more than",
    ];
    if let Some(k) = AUTOMATIC.iter().find(|k| lower.contains(*k)) {
        let idx = lower.find(k).unwrap_or(0);
        return (
            OpportunityStatus::AutomaticDistribution,
            "automatic".into(),
            sentence_around(text, idx, idx + k.len()),
        );
    }
    const PENDING: &[&str] = &[
        "proposed plan of distribution",
        "plan of distribution",
        "fund administrator",
        "distribution agent",
        "fair fund",
        "claims process will",
        "will be announced",
        "claims process has not",
        "administrator will",
    ];
    if let Some(k) = PENDING.iter().find(|k| lower.contains(*k)) {
        let idx = lower.find(k).unwrap_or(0);
        return (
            OpportunityStatus::AwaitingAdministrator,
            "unknown".into(),
            sentence_around(text, idx, idx + k.len()),
        );
    }
    if lower.contains("file a claim") || lower.contains("claim form") || lower.contains("submit a claim") {
        let idx = lower.find("claim").unwrap_or(0);
        return (OpportunityStatus::Open, "online_form".into(), sentence_around(text, idx, idx + 5));
    }
    (OpportunityStatus::Unknown, "unknown".into(), String::new())
}

/// Extracts "$9.8 million"/"$1,234.56" phrases as a described (not guaranteed) benefit.
pub fn find_benefit(text: &str) -> Benefit {
    static MONEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$\s?\d[\d,]*(?:\.\d+)?\s*(?:million|billion)?").unwrap());
    match MONEY.find(text) {
        Some(m) => Benefit {
            amount_known: false,
            description: Some(format!(
                "Source mentions {}; individual amounts vary and are not guaranteed.",
                m.as_str().trim()
            )),
        },
        None => Benefit {
            amount_known: false,
            description: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_and_class_period_detection() {
        let t = "Eligible consumers can file a claim online by March 3, 2027. The class covers purchases made between January 2015 and December 2024.";
        let (d, s) = find_deadline(t).unwrap();
        assert_eq!(d, "2027-03-03");
        assert!(s.contains("file a claim"));
        let (a, b, _) = find_class_period(t).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("2015-01-01", "2024-12-31"));
        assert!(find_deadline("If you get a check, please cash it within 90 days.").is_none());
    }

    #[test]
    fn status_inference() {
        let today = parse_date("2026-10-19").unwrap();
        assert_eq!(
            infer_status("The FTC is sending 65,022 checks totaling more than $9.8 million", None, today).0,
            OpportunityStatus::AutomaticDistribution
        );
        assert_eq!(infer_status("x", Some("2027-01-01"), today).0, OpportunityStatus::Open);
        assert_eq!(infer_status("x", Some("2020-01-01"), today).0, OpportunityStatus::PastDeadline);
        assert_eq!(
            infer_status("The Commission published a notice of the proposed plan of distribution", None, today).0,
            OpportunityStatus::AwaitingAdministrator
        );
        assert_eq!(infer_status("nothing here", None, today).0, OpportunityStatus::Unknown);
    }

    #[test]
    fn absolute_urls_reject_schemes() {
        assert_eq!(absolute("https://www.ftc.gov/a/b", "/x").unwrap(), "https://www.ftc.gov/x");
        assert!(absolute("https://www.ftc.gov/", "javascript:alert(1)").is_none());
    }
}
