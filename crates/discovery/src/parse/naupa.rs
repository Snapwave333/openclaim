//! NAUPA directory of official unclaimed-property programs.

use super::*;
use scraper::{Html, Selector};

pub const INDEX_URL: &str = "https://unclaimed.org/search/";

pub const US_STATES: &[(&str, &str)] = &[
    ("AL", "Alabama"), ("AK", "Alaska"), ("AZ", "Arizona"), ("AR", "Arkansas"), ("CA", "California"), ("CO", "Colorado"),
    ("CT", "Connecticut"), ("DE", "Delaware"), ("DC", "District of Columbia"), ("FL", "Florida"), ("GA", "Georgia"),
    ("HI", "Hawaii"), ("ID", "Idaho"), ("IL", "Illinois"), ("IN", "Indiana"), ("IA", "Iowa"), ("KS", "Kansas"),
    ("KY", "Kentucky"), ("LA", "Louisiana"), ("ME", "Maine"), ("MD", "Maryland"), ("MA", "Massachusetts"),
    ("MI", "Michigan"), ("MN", "Minnesota"), ("MS", "Mississippi"), ("MO", "Missouri"), ("MT", "Montana"),
    ("NE", "Nebraska"), ("NV", "Nevada"), ("NH", "New Hampshire"), ("NJ", "New Jersey"), ("NM", "New Mexico"),
    ("NY", "New York"), ("NC", "North Carolina"), ("ND", "North Dakota"), ("OH", "Ohio"), ("OK", "Oklahoma"),
    ("OR", "Oregon"), ("PA", "Pennsylvania"), ("RI", "Rhode Island"), ("SC", "South Carolina"), ("SD", "South Dakota"),
    ("TN", "Tennessee"), ("TX", "Texas"), ("UT", "Utah"), ("VT", "Vermont"), ("VA", "Virginia"), ("WA", "Washington"),
    ("WV", "West Virginia"), ("WI", "Wisconsin"), ("WY", "Wyoming"),
];

pub const US_TERRITORIES: &[(&str, &str)] = &[("PR", "Puerto Rico"), ("GU", "Guam"), ("VI", "U.S. Virgin Islands"), ("AS", "American Samoa")];

pub const CANADA: &[(&str, &str)] = &[
    ("CA-AB", "Alberta"), ("CA-BC", "British Columbia"), ("CA-QC", "Quebec"), ("CA-NB", "New Brunswick"), ("CA-BOC", "Bank of Canada"),
];

/// Federal and multi-state entries are not scraped: they are policy-reviewed constants.
pub fn fixed_programs() -> Vec<ParsedProgram> {
    vec![
        ParsedProgram { jurisdiction: "US-MISSINGMONEY".into(), name: "MissingMoney.com (NAUPA-endorsed multi-state search)".into(), official_url: "https://www.missingmoney.com/".into(), kind: "multi_state".into() },
        ParsedProgram { jurisdiction: "US-NAUPA".into(), name: "NAUPA state-by-state directory".into(), official_url: "https://unclaimed.org/search/".into(), kind: "multi_state".into() },
        ParsedProgram { jurisdiction: "US-USAGOV".into(), name: "USAGov: unclaimed money from the government".into(), official_url: "https://www.usa.gov/unclaimed-money".into(), kind: "federal".into() },
    ]
}

fn code_for(name: &str) -> Option<(String, &'static str)> {
    let n = name.trim();
    if let Some((c, _)) = US_STATES.iter().find(|(_, s)| s.eq_ignore_ascii_case(n)) {
        return Some(((*c).to_string(), "us_state"));
    }
    if let Some((c, _)) = US_TERRITORIES.iter().find(|(code, s)| s.eq_ignore_ascii_case(n) || (n.eq_ignore_ascii_case("Virgin Islands") && *code == "VI")) {
        return Some(((*c).to_string(), "us_territory"));
    }
    if let Some((c, _)) = CANADA.iter().find(|(_, s)| s.eq_ignore_ascii_case(n)) {
        return Some(((*c).to_string(), "canada"));
    }
    let slug: String = n.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase();
    (!slug.is_empty()).then(|| (format!("OTHER-{slug}"), "other"))
}

pub fn parse_directory(html: &str, base_url: &str) -> Vec<ParsedProgram> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("div.elementor-widget-text-editor a[href], div.entry-content a[href]").expect("static selector");
    let mut out: Vec<ParsedProgram> = Vec::new();
    for a in doc.select(&sel) {
        let name = element_text(a);
        let Some(href) = a.value().attr("href") else { continue };
        let Some(url) = absolute(base_url, href) else { continue };
        let Some((code, kind)) = code_for(&name) else { continue };
        if out.iter().any(|p| p.jurisdiction == code) {
            continue;
        }
        if !openclaim_security::netpolicy::is_safe_display_link(&url) {
            continue;
        }
        out.push(ParsedProgram { jurisdiction: code, name, official_url: url, kind: kind.into() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../../../../test-fixtures/html/naupa_search.html");

    #[test]
    fn all_states_and_dc_present() {
        let programs = parse_directory(PAGE, INDEX_URL);
        for (code, name) in US_STATES {
            assert!(programs.iter().any(|p| p.jurisdiction == *code), "missing {name}");
        }
        let ca = programs.iter().find(|p| p.jurisdiction == "CA").unwrap();
        assert_eq!(ca.official_url, "https://sco.ca.gov/upd_msg.html");
        assert!(programs.iter().any(|p| p.kind == "canada"));
    }
}
