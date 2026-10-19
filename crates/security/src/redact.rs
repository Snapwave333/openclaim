//! Redaction of sensitive identifiers for logs, audit records and agent-facing output.

use regex::Regex;
use std::sync::LazyLock;

static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(), "[REDACTED-SSN]"),
        (Regex::new(r"\b(?:\d[ -]?){13,19}\b").unwrap(), "[REDACTED-NUMBER]"),
        (
            Regex::new(r"(?i)\b(?:routing|account|acct|iban)\s*(?:no\.?|number|#)?\s*[:#]?\s*[A-Z0-9-]{4,}\b").unwrap(),
            "[REDACTED-ACCOUNT]",
        ),
        (Regex::new(r"\b\d{9}\b").unwrap(), "[REDACTED-ID]"),
        (Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b").unwrap(), "[REDACTED-EMAIL]"),
        (Regex::new(r"\(?\b\d{3}\)?[-. ]\d{3}[-. ]\d{4}\b").unwrap(), "[REDACTED-PHONE]"),
    ]
});

static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| vec![Regex::new(r"\boc[a-z]?_[A-Za-z0-9_-]{20,}\b").unwrap()]);

/// Redacts government identifiers, account and card numbers, emails and phone numbers.
pub fn redact_text(input: &str) -> String {
    let mut out = redact_secrets(input);
    for (re, replacement) in PATTERNS.iter() {
        out = re.replace_all(&out, *replacement).into_owned();
    }
    out
}

/// Redacts only OpenClaim credentials (session/agent tokens). Used for logs.
pub fn redact_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for re in SECRET_PATTERNS.iter() {
        out = re.replace_all(&out, "[REDACTED-TOKEN]").into_owned();
    }
    out
}

/// Recursively redacts string values and drops keys that name secrets.
pub fn redact_json(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    const SECRET_KEYS: &[&str] = &[
        "passphrase",
        "password",
        "token",
        "agent_token",
        "session",
        "secret",
        "ssn",
        "account_number",
        "routing_number",
    ];
    match v {
        Value::String(s) => Value::String(redact_text(s)),
        Value::Array(a) => Value::Array(a.iter().map(redact_json).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let lower = k.to_lowercase();
                    if SECRET_KEYS.iter().any(|s| lower == *s || lower.ends_with(&format!("_{s}"))) {
                        (k.clone(), Value::String("[REDACTED]".into()))
                    } else {
                        (k.clone(), redact_json(v))
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Keeps only the first letters of an address-like string for agent display.
pub fn mask_address(addr: &str) -> String {
    let first_line = addr.lines().next().unwrap_or("");
    let visible: String = first_line.chars().take(4).collect();
    format!("{visible}… [address hidden]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_identifiers() {
        let s = "SSN 123-45-6789, card 4111 1111 1111 1111, mail me@x.org, call 555-123-4567, routing number 021000021";
        let r = redact_text(s);
        assert!(!r.contains("6789"));
        assert!(!r.contains("4111"));
        assert!(!r.contains("me@x.org"));
        assert!(!r.contains("4567"));
        assert!(!r.contains("021000021"));
    }

    #[test]
    fn redacts_tokens_and_secret_keys() {
        let t = crate::tokens::new_token("oca");
        assert!(!redact_secrets(&format!("token={t}")).contains(&t[5..]));
        let j = redact_json(&serde_json::json!({"agent_token": "abc", "note": "ok"}));
        assert_eq!(j["agent_token"], "[REDACTED]");
        assert_eq!(j["note"], "ok");
    }
}
