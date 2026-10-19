//! Outbound network policy: defends connectors against SSRF and redirects to
//! non-allowlisted hosts (MDR §12).

use std::net::IpAddr;
use url::{Host, Url};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlRejection {
    Unparseable,
    Scheme(String),
    Credentials,
    IpLiteral,
    Port(u16),
    HostNotAllowed(String),
    NoHost,
}

impl std::fmt::Display for UrlRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UrlRejection::Unparseable => write!(f, "URL could not be parsed"),
            UrlRejection::Scheme(s) => write!(f, "scheme '{s}' is not allowed (https only)"),
            UrlRejection::Credentials => write!(f, "URLs with embedded credentials are not allowed"),
            UrlRejection::IpLiteral => write!(f, "IP-literal hosts are not allowed"),
            UrlRejection::Port(p) => write!(f, "port {p} is not allowed"),
            UrlRejection::HostNotAllowed(h) => write!(f, "host '{h}' is not on the connector allowlist"),
            UrlRejection::NoHost => write!(f, "URL has no host"),
        }
    }
}

/// Is `host` equal to, or a subdomain of, an allowlisted domain?
pub fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allowlist.iter().any(|a| {
        let a = a.trim_end_matches('.').to_ascii_lowercase();
        host == a || host.ends_with(&format!(".{a}"))
    })
}

pub fn check_url(raw: &str, allowlist: &[String]) -> Result<Url, UrlRejection> {
    let url = Url::parse(raw).map_err(|_| UrlRejection::Unparseable)?;
    if url.scheme() != "https" {
        return Err(UrlRejection::Scheme(url.scheme().to_string()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlRejection::Credentials);
    }
    match url.host() {
        None => return Err(UrlRejection::NoHost),
        Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => return Err(UrlRejection::IpLiteral),
        Some(Host::Domain(d)) => {
            if !host_allowed(d, allowlist) {
                return Err(UrlRejection::HostNotAllowed(d.to_string()));
            }
        }
    }
    if let Some(p) = url.port() {
        if p != 443 {
            return Err(UrlRejection::Port(p));
        }
    }
    Ok(url)
}

/// Addresses a public connector must never reach.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0b1100_0000) == 64)
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00
                || (seg[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| is_forbidden_ip(IpAddr::V4(v4)))
        }
    }
}

/// Links shown to users for manual follow-up: https/http only, no credentials, no IP literals.
pub fn is_safe_display_link(raw: &str) -> bool {
    match Url::parse(raw) {
        Ok(u) => {
            matches!(u.scheme(), "https" | "http")
                && u.username().is_empty()
                && u.password().is_none()
                && matches!(u.host(), Some(Host::Domain(_)))
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow() -> Vec<String> {
        vec!["ftc.gov".into(), "sec.gov".into()]
    }

    #[test]
    fn rejects_malicious_urls() {
        assert!(check_url("https://www.ftc.gov/enforcement/refunds", &allow()).is_ok());
        assert_eq!(check_url("http://www.ftc.gov/", &allow()), Err(UrlRejection::Scheme("http".into())));
        assert_eq!(check_url("https://127.0.0.1/", &allow()), Err(UrlRejection::IpLiteral));
        assert_eq!(check_url("https://[::1]/", &allow()), Err(UrlRejection::IpLiteral));
        assert_eq!(check_url("https://user:pw@www.ftc.gov/", &allow()), Err(UrlRejection::Credentials));
        assert_eq!(check_url("https://ftc.gov.evil.example/", &allow()), Err(UrlRejection::HostNotAllowed("ftc.gov.evil.example".into())));
        assert_eq!(check_url("https://evilftc.gov/", &allow()), Err(UrlRejection::HostNotAllowed("evilftc.gov".into())));
        assert_eq!(check_url("https://www.ftc.gov:8443/", &allow()), Err(UrlRejection::Port(8443)));
        assert!(check_url("file:///etc/passwd", &allow()).is_err());
        assert!(check_url("javascript:alert(1)", &allow()).is_err());
    }

    #[test]
    fn forbidden_ips() {
        for ip in ["10.0.0.1", "192.168.1.1", "127.0.0.1", "169.254.169.254", "100.64.0.1", "::1", "fd00::1", "::ffff:10.0.0.1"] {
            assert!(is_forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(!is_forbidden_ip("23.0.0.1".parse().unwrap()));
    }

    #[test]
    fn display_links() {
        assert!(is_safe_display_link("https://unclaimed.org/search/"));
        assert!(!is_safe_display_link("javascript:alert(1)"));
        assert!(!is_safe_display_link("https://10.0.0.1/"));
    }
}
