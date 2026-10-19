//! Policy-gated HTTP fetching: https-only allowlisted hosts, SSRF-safe DNS,
//! redirect checks, robots.txt compliance, per-host spacing, bounded bodies,
//! conditional requests and failure classification. Never evades blocks.

use openclaim_domain::model::FetchFailure;
use openclaim_security::netpolicy::{check_url, is_forbidden_ip};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, RETRY_AFTER, USER_AGENT};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use texting_robots::Robot;
use tokio::sync::Mutex;

pub const DEFAULT_MAX_BODY: usize = 6 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct FetchPolicy {
    pub user_agent: String,
    pub allowed_hosts: Vec<String>,
    pub min_interval: Duration,
    pub max_body: usize,
    pub timeout: Duration,
    pub offline: bool,
}

#[derive(Debug, Clone)]
pub struct Fetched {
    pub url: String,
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub not_modified: bool,
}

#[derive(Debug, Clone)]
pub struct FetchError {
    pub class: FetchFailure,
    pub message: String,
    pub retry_after_secs: Option<u64>,
}

impl FetchError {
    fn new(class: FetchFailure, message: impl Into<String>) -> Self {
        FetchError {
            class,
            message: message.into(),
            retry_after_secs: None,
        }
    }
}

struct SafeResolver;

impl Resolve for SafeResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let safe: Vec<SocketAddr> = addrs.filter(|a| !is_forbidden_ip(a.ip())).collect();
            if safe.is_empty() {
                return Err(
                    Box::new(std::io::Error::other(format!("{host} resolves only to private/loopback addresses"))) as Box<dyn std::error::Error + Send + Sync>
                );
            }
            Ok(Box::new(safe.into_iter()) as Addrs)
        })
    }
}

pub struct Fetcher {
    client: reqwest::Client,
    policy: FetchPolicy,
    robots: Mutex<HashMap<String, Option<Arc<Robot>>>>,
    last_request: Mutex<HashMap<String, Instant>>,
    pub requests_made: Mutex<u32>,
}

impl Fetcher {
    pub fn new(policy: FetchPolicy) -> Result<Self, String> {
        let allow = policy.allowed_hosts.clone();
        let redirect = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("too many redirects");
            }
            match check_url(attempt.url().as_str(), &allow) {
                Ok(_) => attempt.follow(),
                Err(e) => attempt.error(format!("redirect refused: {e}")),
            }
        });
        let client = reqwest::Client::builder()
            .user_agent(policy.user_agent.clone())
            .https_only(true)
            .redirect(redirect)
            .dns_resolver(Arc::new(SafeResolver))
            .timeout(policy.timeout)
            .connect_timeout(Duration::from_secs(15))
            .gzip(true)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Fetcher {
            client,
            policy,
            robots: Mutex::new(HashMap::new()),
            last_request: Mutex::new(HashMap::new()),
            requests_made: Mutex::new(0),
        })
    }

    pub fn policy(&self) -> &FetchPolicy {
        &self.policy
    }

    async fn pace(&self, host: &str, extra_delay: Option<Duration>) {
        let min = extra_delay.map_or(self.policy.min_interval, |d| d.max(self.policy.min_interval));
        let wait = {
            let last = self.last_request.lock().await;
            last.get(host).map(|t| min.saturating_sub(t.elapsed()))
        };
        if let Some(w) = wait
            && !w.is_zero()
        {
            tokio::time::sleep(w).await;
        }
        self.last_request.lock().await.insert(host.to_string(), Instant::now());
        *self.requests_made.lock().await += 1;
    }

    async fn robots_for(&self, host: &str) -> Option<Arc<Robot>> {
        if let Some(r) = self.robots.lock().await.get(host) {
            return r.clone();
        }
        self.pace(host, None).await;
        let url = format!("https://{host}/robots.txt");
        let fetched = self.client.get(&url).send().await;
        let robot = match fetched {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if (400..500).contains(&status) {
                    None
                } else if status >= 500 {
                    Robot::new(&self.policy.user_agent, b"User-agent: *\nDisallow: /\n").ok().map(Arc::new)
                } else {
                    let body = read_bounded(resp, 512 * 1024).await.unwrap_or_default();
                    Robot::new(&self.policy.user_agent, &body).ok().map(Arc::new)
                }
            }
            Err(_) => None,
        };
        self.robots.lock().await.insert(host.to_string(), robot.clone());
        robot
    }

    /// Fetches one allowlisted URL. `conditional` carries the stored ETag / Last-Modified.
    pub async fn get(&self, url: &str, conditional: Option<(Option<String>, Option<String>)>) -> Result<Fetched, FetchError> {
        if self.policy.offline {
            return Err(FetchError::new(FetchFailure::Offline, "offline mode: no network requests are made"));
        }
        let parsed = check_url(url, &self.policy.allowed_hosts).map_err(|e| FetchError::new(FetchFailure::PolicyBlocked, e.to_string()))?;
        let host = parsed.host_str().unwrap_or_default().to_string();
        let robot = self.robots_for(&host).await;
        let mut crawl_delay = None;
        if let Some(r) = &robot {
            if !r.allowed(url) {
                return Err(FetchError::new(
                    FetchFailure::RobotsDisallowed,
                    format!("robots.txt on {host} disallows {}", parsed.path()),
                ));
            }
            crawl_delay = r.delay.map(|d| Duration::from_secs_f32(d.min(60.0)));
        }
        self.pace(&host, crawl_delay).await;
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&self.policy.user_agent).map_err(|e| FetchError::new(FetchFailure::PolicyBlocked, e.to_string()))?,
        );
        if let Some((etag, lm)) = conditional {
            if let Some(e) = etag.and_then(|e| HeaderValue::from_str(&e).ok()) {
                headers.insert(IF_NONE_MATCH, e);
            }
            if let Some(l) = lm.and_then(|l| HeaderValue::from_str(&l).ok()) {
                headers.insert(IF_MODIFIED_SINCE, l);
            }
        }
        let resp = self.client.get(parsed.clone()).headers(headers).send().await.map_err(|e| {
            let class = if e.is_timeout() {
                FetchFailure::NetworkError
            } else if e.is_redirect() {
                FetchFailure::PolicyBlocked
            } else {
                FetchFailure::NetworkError
            };
            FetchError::new(class, format!("request to {host} failed: {e}"))
        })?;
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
        let last_modified = resp.headers().get("last-modified").and_then(|v| v.to_str().ok()).map(str::to_string);
        let retry_after = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if status == 304 {
            return Ok(Fetched {
                url: final_url,
                status,
                body: Vec::new(),
                etag,
                last_modified,
                not_modified: true,
            });
        }
        let body = read_bounded(resp, self.policy.max_body)
            .await
            .map_err(|m| FetchError::new(FetchFailure::HttpError, m))?;
        if status == 429 {
            return Err(FetchError {
                class: FetchFailure::RateLimited,
                message: format!("{host} rate-limited the request (429)"),
                retry_after_secs: retry_after.or(Some(600)),
            });
        }
        if status == 403 || status == 503 || status == 401 {
            let text = String::from_utf8_lossy(&body[..body.len().min(20_000)]).to_lowercase();
            let bot = [
                "automated request",
                "captcha",
                "access denied",
                "rate threshold",
                "bot",
                "cf-chl",
                "challenge",
                "fair access",
            ]
            .iter()
            .any(|k| text.contains(k));
            let class = if bot { FetchFailure::AntiBotBlock } else { FetchFailure::HttpError };
            return Err(FetchError {
                class,
                message: format!(
                    "{host} answered HTTP {status}{}",
                    if bot {
                        " with an anti-automation page; OpenClaim does not bypass it"
                    } else {
                        ""
                    }
                ),
                retry_after_secs: retry_after,
            });
        }
        if !(200..300).contains(&status) {
            return Err(FetchError {
                class: FetchFailure::HttpError,
                message: format!("{host} answered HTTP {status}"),
                retry_after_secs: retry_after,
            });
        }
        Ok(Fetched {
            url: final_url,
            status,
            body,
            etag,
            last_modified,
            not_modified: false,
        })
    }
}

async fn read_bounded(resp: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    use futures::StreamExt;
    if let Some(len) = resp.content_length()
        && len as usize > max
    {
        return Err(format!("response body of {len} bytes exceeds the {max}-byte limit"));
    }
    let mut out = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if out.len() + chunk.len() > max {
            return Err(format!("response body exceeds the {max}-byte limit"));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

pub fn default_user_agent(contact: Option<&str>) -> String {
    let base = format!(
        "OpenClaim/{} (+https://github.com/openclaim/openclaim; free open-source settlement assistant)",
        openclaim_domain::VERSION
    );
    match contact {
        Some(c) if !c.trim().is_empty() => format!("{base} {}", c.trim()),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn offline_and_policy_rejections_need_no_network() {
        let f = Fetcher::new(FetchPolicy {
            user_agent: "t".into(),
            allowed_hosts: vec!["ftc.gov".into()],
            min_interval: Duration::from_millis(500),
            max_body: 1024,
            timeout: Duration::from_secs(5),
            offline: true,
        })
        .unwrap();
        assert_eq!(f.get("https://www.ftc.gov/", None).await.unwrap_err().class, FetchFailure::Offline);
        let f = Fetcher::new(FetchPolicy {
            user_agent: "t".into(),
            allowed_hosts: vec!["ftc.gov".into()],
            min_interval: Duration::from_millis(500),
            max_body: 1024,
            timeout: Duration::from_secs(5),
            offline: false,
        })
        .unwrap();
        assert_eq!(f.get("https://evil.example/", None).await.unwrap_err().class, FetchFailure::PolicyBlocked);
        assert_eq!(f.get("http://www.ftc.gov/", None).await.unwrap_err().class, FetchFailure::PolicyBlocked);
        assert_eq!(f.get("https://127.0.0.1/", None).await.unwrap_err().class, FetchFailure::PolicyBlocked);
    }
}
