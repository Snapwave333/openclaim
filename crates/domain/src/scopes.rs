//! Capability scopes for local principals (MDR §20.1).
//!
//! There is deliberately no scope that allows official claim submission.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How dangerous an operation is. Drives authorization for agent principals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Reads local or catalog data. Never mutates.
    Read,
    /// Mutates local metadata. Agents need the scope.
    Write,
    /// Sensitive mutation. Agents need the scope *and* a human approval bound to the exact action.
    Elevated,
    /// Only a human owner session may perform it (approvals, principal management, wipe).
    OwnerOnly,
}

impl Effect {
    pub fn is_mutation(self) -> bool {
        !matches!(self, Effect::Read)
    }
}

pub const PUBLIC_RESEARCH_SCOPES: &[&str] = &[
    "discover.read",
    "opportunities.read",
    "sources.read",
    "property.read",
    "schema.read",
    "daemon.read",
];

pub const ALL_SCOPES: &[(&str, &str)] = &[
    ("discover.read", "Search and browse the opportunity catalog"),
    ("opportunities.read", "Read opportunity details, rules and provenance"),
    ("opportunities.write", "Bookmark, hide, report or curate opportunity metadata"),
    ("sources.read", "Read source registry, health and history"),
    ("sources.refresh", "Start compliant, rate-limited source refresh jobs"),
    ("sources.manage", "Register, disable or import snapshots for sources"),
    ("property.read", "Read the official unclaimed-property directory"),
    ("property.track", "Track unclaimed-property research for a person"),
    ("match.evaluate", "Run deterministic eligibility evaluation"),
    ("jobs.read", "Read background job state"),
    ("jobs.manage", "Enqueue, cancel or retry jobs"),
    ("alerts.read", "Read deadline reminders"),
    ("alerts.write", "Create, snooze or dismiss reminders"),
    ("people.read", "Read claimant profiles (redacted by default)"),
    ("people.read_sensitive", "Decrypt optional profile details (addresses, emails, phones)"),
    ("people.modify", "Create or change claimant profiles"),
    ("facts.propose", "Propose untrusted facts for human review"),
    ("facts.review", "Confirm or reject proposed facts"),
    ("documents.metadata", "List document metadata"),
    ("documents.read", "Read bounded, redacted document excerpts"),
    ("documents.import", "Import local files into the evidence vault"),
    ("documents.delete", "Delete documents from the evidence vault"),
    ("claims.read", "Read claim tracking records"),
    ("claims.draft", "Prepare local claim packages and checklists"),
    ("claims.track", "Record claim status changes"),
    ("workflows.run", "Plan and run allowlisted workflows"),
    ("data.export", "Export local records"),
    ("data.wipe", "Wipe local data (owner only)"),
    ("agents.manage", "Register, authorize and revoke principals (owner only)"),
    ("approvals.decide", "Approve or deny pending actions (owner only)"),
    ("audit.read", "Read the audit trail"),
    ("config.write", "Change configuration"),
    ("schema.read", "Read command and tool schemas"),
    ("daemon.read", "Read daemon status and health"),
    ("daemon.manage", "Stop or restart the daemon"),
    ("extension.prefill", "Fetch user-approved prefill packages (browser extension)"),
];

pub fn is_known_scope(scope: &str) -> bool {
    let base = scope.split(':').next().unwrap_or(scope);
    ALL_SCOPES.iter().any(|(s, _)| *s == base)
}

/// Scopes that can never be granted to an agent principal.
pub const OWNER_ONLY_SCOPES: &[&str] = &["data.wipe", "agents.manage", "approvals.decide"];

/// Does `granted` satisfy `required`? Supports resource-narrowed grants such as
/// `documents.read:<document-id>` matching a requirement for that resource.
pub fn scope_satisfies(granted: &str, required: &str, resource: Option<&str>) -> bool {
    if granted == required {
        return true;
    }
    if let Some((base, res)) = granted.split_once(':') {
        return base == required && resource == Some(res);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowed_scopes_only_match_their_resource() {
        assert!(scope_satisfies("documents.read", "documents.read", Some("d1")));
        assert!(scope_satisfies("documents.read:d1", "documents.read", Some("d1")));
        assert!(!scope_satisfies("documents.read:d1", "documents.read", Some("d2")));
        assert!(!scope_satisfies("documents.read:d1", "documents.read", None));
        assert!(!scope_satisfies("discover.read", "documents.read", None));
    }

    #[test]
    fn no_submission_scope_exists() {
        assert!(!ALL_SCOPES.iter().any(|(s, _)| s.contains("submit")));
    }
}
