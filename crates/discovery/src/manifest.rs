use openclaim_domain::model::AuthorityTier;
use openclaim_domain::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    FtcRefunds,
    SecDistributions,
    NaupaDirectory,
    Feed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthRequirement {
    None,
    /// The site's fair-access policy requires a User-Agent that names a contact (SEC.gov).
    DeclaredContactUserAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RateLimit {
    /// Minimum spacing between requests to the same host.
    pub min_interval_ms: u64,
    /// Upper bound on requests per refresh run.
    pub max_requests_per_run: u32,
}

/// Provider gate (MDR §3). A manifest that fails validation cannot be enabled.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectorManifest {
    pub id: String,
    pub name: String,
    pub connector: ConnectorKind,
    pub url: String,
    pub authority_tier: AuthorityTier,
    /// Must be exactly "free".
    pub price: String,
    pub auth_requirement: AuthRequirement,
    /// Must be "respect".
    pub robots_policy: String,
    pub terms_url: String,
    /// YYYY-MM-DD the terms were last reviewed by a human.
    pub terms_review_date: String,
    pub rate_limit: RateLimit,
    /// False means the site forbids or blocks automation: only user-provided snapshots are accepted.
    pub automation_allowed: bool,
    pub allowed_hosts: Vec<String>,
    pub freshness_ttl_hours: u32,
    #[serde(default)]
    pub follow_detail_pages: bool,
    #[serde(default)]
    pub max_detail_pages: u32,
    #[serde(default)]
    pub notes: String,
}

impl ConnectorManifest {
    pub fn validate(&self) -> OcResult<()> {
        if self.price != "free" {
            return Err(OcError::policy(format!("connector '{}' is not free (price={}); refusing", self.id, self.price)));
        }
        if self.robots_policy != "respect" {
            return Err(OcError::policy(format!("connector '{}' must respect robots rules", self.id)));
        }
        if self.allowed_hosts.is_empty() {
            return Err(OcError::invalid(format!("connector '{}' needs allowed_hosts", self.id)));
        }
        openclaim_domain::dates::parse_date(&self.terms_review_date)?;
        openclaim_security::netpolicy::check_url(&self.url, &self.allowed_hosts)
            .map_err(|e| OcError::invalid(format!("connector '{}' url rejected: {e}", self.id)))?;
        if self.rate_limit.min_interval_ms < 500 {
            return Err(OcError::policy("min_interval_ms must be at least 500"));
        }
        if self.id.is_empty() || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(OcError::invalid("connector id must be [A-Za-z0-9_-]+"));
        }
        Ok(())
    }

    pub fn parse(json: &str) -> OcResult<Self> {
        let m: ConnectorManifest = serde_json::from_str(json)?;
        m.validate()?;
        Ok(m)
    }
}

pub const BUNDLED_MANIFESTS: &[&str] = &[
    include_str!("../../../source-manifests/ftc-refunds.json"),
    include_str!("../../../source-manifests/sec-distributions.json"),
    include_str!("../../../source-manifests/naupa-directory.json"),
];

pub fn bundled() -> Vec<ConnectorManifest> {
    BUNDLED_MANIFESTS
        .iter()
        .map(|j| ConnectorManifest::parse(j).expect("bundled manifests are valid"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_manifests_validate_and_are_free() {
        let all = bundled();
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|m| m.price == "free" && m.robots_policy == "respect"));
    }

    #[test]
    fn paid_manifest_is_refused() {
        let mut m = bundled().remove(0);
        m.price = "subscription".into();
        assert_eq!(m.validate().unwrap_err().code, openclaim_domain::ErrorCode::PolicyBlocked);
    }
}
