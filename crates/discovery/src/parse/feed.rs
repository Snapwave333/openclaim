//! Generic RSS/Atom feed connector. Feed items are Tier B discovery leads and
//! enter quarantine as tips; they never become verified opportunities on their own.

use super::ParsedTip;
use openclaim_security::sanitize::{clean_text, html_fragment_to_text};

pub fn parse_feed(bytes: &[u8]) -> Vec<ParsedTip> {
    let Ok(feed) = feed_rs::parser::parse(bytes) else { return Vec::new() };
    feed.entries
        .into_iter()
        .filter_map(|e| {
            let link = e.links.first().map(|l| l.href.clone())?;
            if !openclaim_security::netpolicy::is_safe_display_link(&link) {
                return None;
            }
            let title = e.title.map(|t| clean_text(&t.content, 300)).unwrap_or_else(|| link.clone());
            let summary = e
                .summary
                .map(|s| clean_text(&html_fragment_to_text(&s.content), 1000))
                .or_else(|| e.content.and_then(|c| c.body).map(|b| clean_text(&html_fragment_to_text(&b), 1000)));
            let published = e.published.or(e.updated).map(|d| d.to_rfc3339());
            Some(ParsedTip {
                url: link,
                title,
                published,
                summary,
            })
        })
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_items() {
        let rss = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
        <item><title>Settlement reached in Widget case</title><link>https://example.test/a</link><description>&lt;p&gt;Claims &lt;script&gt;x()&lt;/script&gt; open&lt;/p&gt;</description></item>
        <item><title>bad</title><link>javascript:alert(1)</link></item>
        </channel></rss>"#;
        let tips = parse_feed(rss.as_bytes());
        assert_eq!(tips.len(), 1);
        assert_eq!(tips[0].summary.as_deref(), Some("Claims open"));
    }
}
