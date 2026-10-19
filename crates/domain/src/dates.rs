use crate::error::{OcError, OcResult};
use time::macros::format_description;
use time::{Date, OffsetDateTime};

pub fn parse_date(s: &str) -> OcResult<Date> {
    Date::parse(s, format_description!("[year]-[month]-[day]")).map_err(|_| OcError::invalid(format!("'{s}' is not a YYYY-MM-DD date")))
}

pub fn format_date(d: Date) -> String {
    d.format(format_description!("[year]-[month]-[day]")).expect("date formats")
}

pub fn now_rfc3339() -> String {
    format_ts(OffsetDateTime::now_utc())
}

pub fn format_ts(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339).expect("timestamp formats")
}

pub fn parse_ts(s: &str) -> OcResult<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).map_err(|_| OcError::invalid(format!("'{s}' is not an RFC 3339 timestamp")))
}

pub fn today_utc() -> Date {
    OffsetDateTime::now_utc().date()
}

/// Inclusive date interval. `None` on either side means open-ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: Option<Date>,
    pub end: Option<Date>,
}

impl Span {
    pub fn overlaps(&self, other: &Span) -> bool {
        let starts_before_other_ends = match (self.start, other.end) {
            (Some(s), Some(e)) => s <= e,
            _ => true,
        };
        let ends_after_other_starts = match (self.end, other.start) {
            (Some(e), Some(s)) => e >= s,
            _ => true,
        };
        starts_before_other_ends && ends_after_other_starts
    }

    pub fn contains(&self, d: Date) -> bool {
        self.start.is_none_or(|s| s <= d) && self.end.is_none_or(|e| d <= e)
    }
}

/// Month name → number, for parsing dates written in prose ("March 3, 2027").
pub fn month_from_name(name: &str) -> Option<u8> {
    let n = name.trim().trim_end_matches('.').to_ascii_lowercase();
    let months = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    months
        .iter()
        .position(|m| *m == n || (n.len() >= 3 && m.starts_with(&n)))
        .map(|i| (i + 1) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_overlap_inclusively() {
        let a = Span {
            start: Some(parse_date("2020-01-01").unwrap()),
            end: Some(parse_date("2020-12-31").unwrap()),
        };
        let b = Span {
            start: Some(parse_date("2020-12-31").unwrap()),
            end: None,
        };
        let c = Span {
            start: Some(parse_date("2021-01-01").unwrap()),
            end: None,
        };
        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn month_names() {
        assert_eq!(month_from_name("March"), Some(3));
        assert_eq!(month_from_name("Sept."), Some(9));
        assert_eq!(month_from_name("Foo"), None);
    }
}
