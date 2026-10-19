//! Deterministic, on-device document extraction. No network, no models.
//!
//! Extraction output is a *suggestion* source: facts found here are stored as
//! proposals and only become usable after a human confirms them.

use openclaim_domain::dates::month_from_name;
use openclaim_domain::{OcError, OcResult};
use openclaim_security::sanitize::{clean_text, html_fragment_to_text};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

pub const MAX_IMPORT_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_TEXT_CHARS: usize = 2_000_000;
pub const MAX_MBOX_MESSAGES: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocType {
    Pdf,
    Image,
    Csv,
    Eml,
    Mbox,
    Text,
}

impl DocType {
    pub fn as_str(self) -> &'static str {
        match self {
            DocType::Pdf => "pdf",
            DocType::Image => "image",
            DocType::Csv => "csv",
            DocType::Eml => "eml",
            DocType::Mbox => "mbox",
            DocType::Text => "text",
        }
    }

    pub fn mime(self, ext: &str) -> &'static str {
        match self {
            DocType::Pdf => "application/pdf",
            DocType::Image => match ext {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "tif" | "tiff" => "image/tiff",
                _ => "image/octet-stream",
            },
            DocType::Csv => "text/csv",
            DocType::Eml => "message/rfc822",
            DocType::Mbox => "application/mbox",
            DocType::Text => "text/plain",
        }
    }
}

/// Classifies by magic bytes first, then extension. Unknown/active formats are refused.
pub fn detect(bytes: &[u8], filename: &str) -> OcResult<DocType> {
    let ext = extension(filename);
    if bytes.starts_with(b"%PDF-") {
        return Ok(DocType::Pdf);
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) || bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return Ok(DocType::Image);
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"MZ") || bytes.starts_with(&[0x1F, 0x8B]) {
        return Err(OcError::policy("archives and executables are not accepted; import the individual documents instead"));
    }
    let text_like = std::str::from_utf8(&bytes[..bytes.len().min(4096)]).is_ok()
        || bytes.iter().take(4096).filter(|b| **b == 0).count() == 0;
    if !text_like {
        return Err(OcError::invalid(format!("'{filename}' is not a supported document type (PDF, image, CSV, EML, MBOX, text)")));
    }
    Ok(match ext.as_str() {
        "csv" => DocType::Csv,
        "eml" => DocType::Eml,
        "mbox" | "mbx" => DocType::Mbox,
        "html" | "htm" | "svg" | "js" | "exe" | "bat" | "ps1" | "cmd" => {
            return Err(OcError::policy(format!("'.{ext}' files can carry active content and are not accepted; save as PDF or text")));
        }
        _ if bytes.starts_with(b"From ") => DocType::Mbox,
        _ => DocType::Text,
    })
}

pub fn extension(filename: &str) -> String {
    Path::new(filename).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Extraction {
    /// `extracted`, `empty`, `ocr_unavailable`, `failed`.
    pub state: String,
    pub text: String,
    pub note: Option<String>,
    pub classification: String,
    pub suggestions: Vec<FactSuggestion>,
}

/// A deterministic, non-authoritative fact suggestion with its source span.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FactSuggestion {
    pub kind: String,
    pub value: String,
    /// Byte offsets into the extracted text.
    pub span_start: usize,
    pub span_end: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, Default)]
pub struct OcrConfig {
    /// Explicit path to `tesseract`; otherwise PATH is searched.
    pub tesseract: Option<PathBuf>,
    pub enabled: bool,
}

pub fn find_tesseract(cfg: &OcrConfig) -> Option<PathBuf> {
    if let Some(p) = &cfg.tesseract {
        return p.exists().then(|| p.clone());
    }
    let exe = if cfg!(windows) { "tesseract.exe" } else { "tesseract" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect())
        .unwrap_or_default();
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn extract(bytes: &[u8], doc_type: DocType, ocr: &OcrConfig) -> Extraction {
    let (state, raw, note) = match doc_type {
        DocType::Pdf => match pdf_text(bytes) {
            Ok(t) if t.trim().is_empty() => (
                "empty",
                String::new(),
                Some("No embedded text (likely a scanned PDF). Export the pages as PNG and import them for on-device OCR.".to_string()),
            ),
            Ok(t) => ("extracted", t, None),
            Err(e) => ("failed", String::new(), Some(e)),
        },
        DocType::Image => {
            if !ocr.enabled {
                ("ocr_unavailable", String::new(), Some("OCR is turned off in settings (vault.ocr=false).".to_string()))
            } else {
                match find_tesseract(ocr) {
                    None => (
                        "ocr_unavailable",
                        String::new(),
                        Some("Tesseract OCR is not installed. Install the free Tesseract build, or type the details yourself.".to_string()),
                    ),
                    Some(bin) => match run_tesseract(&bin, bytes) {
                        Ok(t) => (if t.trim().is_empty() { "empty" } else { "extracted" }, t, None),
                        Err(e) => ("failed", String::new(), Some(e)),
                    },
                }
            }
        }
        DocType::Eml => match email_text(bytes) {
            Some(t) => ("extracted", t, None),
            None => ("failed", String::new(), Some("could not parse the email".to_string())),
        },
        DocType::Mbox => {
            let (t, n, truncated) = mbox_text(bytes);
            let note = Some(format!(
                "{n} message(s) parsed{}",
                if truncated { format!("; stopped at the {MAX_MBOX_MESSAGES}-message limit") } else { String::new() }
            ));
            ("extracted", t, note)
        }
        DocType::Csv | DocType::Text => ("extracted", String::from_utf8_lossy(bytes).into_owned(), None),
    };
    let text = clean_text(&raw, MAX_TEXT_CHARS);
    let classification = classify(&text).to_string();
    let suggestions = suggest_facts(&text);
    Extraction { state: state.to_string(), text, note, classification, suggestions }
}

fn pdf_text(bytes: &[u8]) -> Result<String, String> {
    let owned = bytes.to_vec();
    // pdf-extract can panic on malformed input; a crafted PDF must not take down the daemon.
    std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&owned))
        .map_err(|_| "the PDF parser rejected this file (malformed or unsupported PDF)".to_string())?
        .map_err(|e| format!("PDF text extraction failed: {e}"))
}

fn run_tesseract(bin: &Path, bytes: &[u8]) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let input = dir.path().join("page.img");
    std::fs::write(&input, bytes).map_err(|e| e.to_string())?;
    let out = Command::new(bin)
        .arg(&input)
        .arg("stdout")
        .output()
        .map_err(|e| format!("could not run tesseract: {e}"))?;
    if !out.status.success() {
        return Err(format!("tesseract failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn email_text(bytes: &[u8]) -> Option<String> {
    let msg = mail_parser::MessageParser::default().parse(bytes)?;
    let mut out = String::new();
    if let Some(from) = msg.from().and_then(|f| f.first()) {
        out.push_str(&format!("From: {} <{}>\n", from.name().unwrap_or(""), from.address().unwrap_or("")));
    }
    if let Some(d) = msg.date() {
        out.push_str(&format!("Date: {}\n", d.to_rfc3339()));
    }
    if let Some(s) = msg.subject() {
        out.push_str(&format!("Subject: {s}\n"));
    }
    out.push('\n');
    if let Some(body) = msg.body_text(0) {
        out.push_str(&body);
    } else if let Some(html) = msg.body_html(0) {
        out.push_str(&html_fragment_to_text(&html));
    }
    Some(out)
}

fn mbox_text(bytes: &[u8]) -> (String, usize, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut count = 0;
    let mut truncated = false;
    let mut current = String::new();
    let flush = |msg: &str, out: &mut String, count: &mut usize| {
        if msg.trim().is_empty() {
            return;
        }
        if let Some(t) = email_text(msg.as_bytes()) {
            *count += 1;
            out.push_str(&format!("----- message {count} -----\n{t}\n"));
        }
    };
    for line in text.split_inclusive('\n') {
        if line.starts_with("From ") && !current.is_empty() {
            flush(&current, &mut out, &mut count);
            current.clear();
            if count >= MAX_MBOX_MESSAGES {
                truncated = true;
                break;
            }
            continue;
        }
        if line.starts_with("From ") && current.is_empty() {
            continue;
        }
        current.push_str(line.strip_prefix('>').filter(|l| l.starts_with("From ")).unwrap_or(line));
    }
    if !truncated {
        flush(&current, &mut out, &mut count);
    }
    (out, count, truncated)
}

pub fn classify(text: &str) -> &'static str {
    let t = text.to_lowercase();
    let has = |words: &[&str]| words.iter().filter(|w| t.contains(*w)).count();
    if has(&["class action", "settlement administrator", "claim form", "class member", "notice of proposed settlement"]) >= 1 {
        "settlement_notice"
    } else if has(&["refund", "ftc", "federal trade commission"]) >= 2 {
        "refund_notice"
    } else if has(&["statement period", "opening balance", "closing balance", "account summary"]) >= 2 {
        "account_statement"
    } else if has(&["receipt", "order total", "subtotal", "order #", "order number", "invoice"]) >= 1 {
        "receipt"
    } else if t.starts_with("from:") || t.contains("\nsubject:") {
        "email"
    } else {
        "other"
    }
}

static NUMERIC_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{4})-(\d{2})-(\d{2})\b|\b(\d{1,2})/(\d{1,2})/(\d{4})\b").unwrap());
static PROSE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)\.?\s+(\d{1,2}),?\s+(\d{4})\b").unwrap()
});
static AMOUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:total|amount|paid|charged)\b[^\n$]{0,30}\$\s?(\d{1,3}(?:,\d{3})*(?:\.\d{2})?)").unwrap());
static MERCHANT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^(?:from|merchant|sold by|seller|store)\s*:\s*([^\n<]{2,60})").unwrap());
static CASE_NO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bcase\s+(?:no\.?|number)\s*:?\s*([0-9]{1,2}:[0-9]{2}-cv-[0-9]{3,6}(?:-[A-Z]{2,4})?|[A-Z0-9-]{5,30})").unwrap());

fn excerpt(text: &str, start: usize, end: usize) -> String {
    let s = text[..start].char_indices().rev().nth(40).map(|(i, _)| i).unwrap_or(0);
    let e = text[end..].char_indices().nth(40).map(|(i, _)| end + i).unwrap_or(text.len());
    text[s..e].replace('\n', " ")
}

/// Deterministic pattern suggestions: dates, amounts, merchant lines, case numbers.
pub fn suggest_facts(text: &str) -> Vec<FactSuggestion> {
    let mut out = Vec::new();
    let mut push = |kind: &str, value: String, start: usize, end: usize| {
        if out.iter().any(|s: &FactSuggestion| s.kind == kind && s.value == value) || out.len() >= 50 {
            return;
        }
        out.push(FactSuggestion { kind: kind.into(), value, span_start: start, span_end: end, excerpt: excerpt(text, start, end) });
    };
    for c in NUMERIC_DATE.captures_iter(text) {
        let m = c.get(0).unwrap();
        let iso = if let (Some(y), Some(mo), Some(d)) = (c.get(1), c.get(2), c.get(3)) {
            format!("{}-{}-{}", y.as_str(), mo.as_str(), d.as_str())
        } else {
            let (mo, d, y) = (&c[4], &c[5], &c[6]);
            format!("{y}-{:0>2}-{:0>2}", mo, d)
        };
        if openclaim_domain::dates::parse_date(&iso).is_ok() {
            push("date", iso, m.start(), m.end());
        }
    }
    for c in PROSE_DATE.captures_iter(text) {
        let m = c.get(0).unwrap();
        if let Some(mo) = month_from_name(&c[1]) {
            let iso = format!("{}-{:02}-{:0>2}", &c[3], mo, &c[2]);
            if openclaim_domain::dates::parse_date(&iso).is_ok() {
                push("date", iso, m.start(), m.end());
            }
        }
    }
    for c in AMOUNT.captures_iter(text) {
        let g = c.get(1).unwrap();
        let cents = parse_amount_minor(g.as_str());
        if let Some(cents) = cents {
            push("amount_minor_usd", cents.to_string(), g.start(), g.end());
        }
    }
    for c in MERCHANT.captures_iter(text) {
        let g = c.get(1).unwrap();
        push("merchant", g.as_str().trim().to_string(), g.start(), g.end());
    }
    for c in CASE_NO.captures_iter(text) {
        let g = c.get(1).unwrap();
        push("case_number", g.as_str().to_string(), g.start(), g.end());
    }
    out
}

/// "1,234.56" → 123456 minor units, without floating point.
pub fn parse_amount_minor(s: &str) -> Option<i64> {
    let s = s.replace(',', "");
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) if f.len() == 2 => (w, f),
        Some(_) => return None,
        None => (s.as_str(), "00"),
    };
    Some(whole.parse::<i64>().ok()? * 100 + frac.parse::<i64>().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_active_and_archive_content() {
        assert!(detect(b"PK\x03\x04....", "a.zip").is_err());
        assert!(detect(b"MZ\x90\x00", "a.exe").is_err());
        assert!(detect(b"<html><script>", "a.html").is_err());
        assert_eq!(detect(b"%PDF-1.7 ...", "x.bin").unwrap(), DocType::Pdf);
        assert_eq!(detect(b"a,b\n1,2\n", "x.csv").unwrap(), DocType::Csv);
    }

    #[test]
    fn suggestions_cite_spans() {
        let text = "Receipt\nMerchant: Acme Widgets\nOrder date: March 3, 2020\nTotal: $1,234.56\nCase No. 2:23-cv-01495";
        let s = suggest_facts(text);
        let get = |k: &str| s.iter().find(|x| x.kind == k).unwrap().value.clone();
        assert_eq!(get("date"), "2020-03-03");
        assert_eq!(get("amount_minor_usd"), "123456");
        assert_eq!(get("merchant"), "Acme Widgets");
        assert_eq!(get("case_number"), "2:23-cv-01495");
        for x in &s {
            let span = &text[x.span_start..x.span_end];
            assert!(x.excerpt.contains(span), "excerpt must contain the cited span");
        }
        assert_eq!(&text[s.iter().find(|x| x.kind == "merchant").unwrap().span_start..][..12], "Acme Widgets");
        assert_eq!(classify(text), "receipt");
    }

    #[test]
    fn email_and_mbox() {
        let eml = b"From: Settlement Admin <info@example-settlement.test>\r\nSubject: Notice of Class Action Settlement\r\nDate: Tue, 1 Sep 2026 10:00:00 +0000\r\n\r\nYou may be a class member. Claim form deadline: December 1, 2026.\r\n";
        let e = extract(eml, DocType::Eml, &OcrConfig::default());
        assert_eq!(e.state, "extracted");
        assert!(e.text.contains("Notice of Class Action Settlement"));
        assert_eq!(e.classification, "settlement_notice");
        let mbox = [b"From a@b Tue Sep  1 10:00:00 2026\n".as_slice(), eml, b"\nFrom c@d Tue Sep  1 11:00:00 2026\n", eml].concat();
        let m = extract(&mbox, DocType::Mbox, &OcrConfig::default());
        assert!(m.note.unwrap().starts_with("2 message"));
    }

    #[test]
    fn malformed_pdf_does_not_panic() {
        let e = extract(b"%PDF-1.4\n garbage \x00\x01", DocType::Pdf, &OcrConfig::default());
        assert_eq!(e.state, "failed");
    }

    #[test]
    fn amounts_use_integers() {
        assert_eq!(parse_amount_minor("12"), Some(1200));
        assert_eq!(parse_amount_minor("0.07"), Some(7));
        assert_eq!(parse_amount_minor("1.5"), None);
    }
}
