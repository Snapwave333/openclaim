use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum $name {
            $(#[serde(rename = $s)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($name::$variant),)+ _ => None }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_enum! {
    /// Lifecycle of an opportunity's claim window.
    OpportunityStatus {
        Open => "open",
        Upcoming => "upcoming",
        AutomaticDistribution => "automatic_distribution",
        PastDeadline => "past_deadline",
        Closed => "closed",
        AwaitingAdministrator => "awaiting_administrator",
        Unknown => "unknown",
    }
}

string_enum! {
    /// Whether a primary source has confirmed the record.
    VerificationState {
        Verified => "verified",
        VerificationPending => "verification_pending",
        Stale => "stale",
        Disputed => "disputed",
    }
}

string_enum! {
    /// Authority tier of a source (MDR §4).
    AuthorityTier {
        A => "A",
        B => "B",
        User => "user",
    }
}

string_enum! {
    OpportunityType {
        GovernmentRefund => "government_refund",
        InvestorDistribution => "investor_distribution",
        ClassAction => "class_action",
        UnclaimedProperty => "unclaimed_property",
        Other => "other",
    }
}

string_enum! {
    ProofRequirement {
        None => "none",
        Optional => "optional",
        Required => "required",
        Unknown => "unknown",
    }
}

string_enum! {
    SourceHealth {
        Healthy => "healthy",
        Degraded => "degraded",
        Blocked => "blocked",
        NeedsConfiguration => "needs_configuration",
        Disabled => "disabled",
        Never => "never_fetched",
    }
}

string_enum! {
    /// Failure classes for connector fetches.
    FetchFailure {
        AntiBotBlock => "anti_bot_block",
        RobotsDisallowed => "robots_disallowed",
        HttpError => "http_error",
        NetworkError => "network_error",
        ParseError => "parse_error",
        PolicyBlocked => "policy_blocked",
        RateLimited => "rate_limited",
        Offline => "offline",
    }
}

string_enum! {
    JobState {
        Queued => "queued",
        Running => "running",
        PausedForApproval => "paused_for_approval",
        Succeeded => "succeeded",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Succeeded | JobState::Failed | JobState::Cancelled)
    }
}

string_enum! {
    ApprovalDecision {
        Pending => "pending",
        Approved => "approved",
        Denied => "denied",
        Expired => "expired",
        Consumed => "consumed",
        Invalidated => "invalidated",
    }
}

string_enum! {
    FactValidation {
        Proposed => "proposed",
        Confirmed => "confirmed",
        Rejected => "rejected",
    }
}

string_enum! {
    /// Trust label attached to every piece of data returned to agents (MDR §19.3).
    TrustLabel {
        UntrustedSourceText => "untrusted_source_text",
        SourceMetadata => "source_metadata",
        DeterministicEvaluation => "deterministic_evaluation",
        AgentProposed => "agent_proposed",
        HumanConfirmed => "human_confirmed",
    }
}

/// Normalizes a case title for deduplication: lowercase alphanumerics only,
/// with common legal noise words removed. Never used on its own to merge.
pub fn normalize_title(title: &str) -> String {
    const NOISE: &[&str] = &[
        "the", "inc", "llc", "ltd", "corp", "corporation", "co", "company", "et", "al", "refunds",
        "refund", "settlement", "program", "v", "vs", "in", "re", "matter", "of", "sec", "ftc",
    ];
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !NOISE.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Dedupe key: normalized title + docket + administrator + key date.
/// Two records only merge when every present component agrees.
pub fn dedupe_key(
    title: &str,
    case_number: Option<&str>,
    administrator: Option<&str>,
    class_period_start: Option<&str>,
) -> String {
    let norm = |s: Option<&str>| {
        s.map(|v| v.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>())
            .unwrap_or_default()
    };
    format!(
        "{}|{}|{}|{}",
        normalize_title(title),
        norm(case_number),
        norm(administrator),
        class_period_start.unwrap_or("")
    )
}

/// Whether two dedupe keys describe the same settlement. Components that are
/// blank on one side do not block a merge, but the title must match and at
/// least one strong identifier (docket or administrator) must match when both
/// sides carry it.
pub fn same_settlement(a: &str, b: &str) -> bool {
    let pa: Vec<&str> = a.split('|').collect();
    let pb: Vec<&str> = b.split('|').collect();
    if pa.len() != 4 || pb.len() != 4 || pa[0].is_empty() || pa[0] != pb[0] {
        return false;
    }
    for i in 1..4 {
        if !pa[i].is_empty() && !pb[i].is_empty() && pa[i] != pb[i] {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_settlement_across_publishers() {
        let a = dedupe_key("Avast Settlement", None, Some("Rust Consulting, Inc."), None);
        let b = dedupe_key("The Avast Refunds", None, Some("rust consulting inc"), None);
        assert!(same_settlement(&a, &b));
    }

    #[test]
    fn same_company_different_settlements_do_not_merge() {
        let a = dedupe_key("Amazon Prime Refunds", Some("2:23-cv-0932"), None, None);
        let b = dedupe_key("Amazon Alexa Refunds", Some("2:23-cv-0811"), None, None);
        assert!(!same_settlement(&a, &b));
        let c = dedupe_key("Amazon Refunds", Some("2:23-cv-0932"), None, None);
        let d = dedupe_key("Amazon Refunds", Some("2:23-cv-0811"), None, None);
        assert!(!same_settlement(&c, &d));
        let e = dedupe_key("Amazon Refunds", None, Some("JND Legal"), None);
        let f = dedupe_key("Amazon Refunds", None, Some("Rust Consulting"), None);
        assert!(!same_settlement(&e, &f));
    }

    #[test]
    fn enums_round_trip() {
        for s in OpportunityStatus::ALL {
            assert_eq!(OpportunityStatus::parse(s.as_str()), Some(*s));
        }
    }
}
