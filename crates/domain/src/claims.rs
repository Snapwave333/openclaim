//! Claim tracking state machine (MDR §5.7). Status changes that assert an
//! external fact (submission, approval, payment) require user confirmation and
//! evidence; nothing is inferred.

use crate::error::{OcError, OcResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Discovered,
    Reviewing,
    PossiblyEligible,
    ProofNeeded,
    Ready,
    OpenedOfficialForm,
    Submitted,
    Pending,
    Approved,
    Denied,
    Paid,
    Withdrawn,
    Expired,
}

impl ClaimStatus {
    pub const ALL: &'static [ClaimStatus] = &[
        ClaimStatus::Discovered,
        ClaimStatus::Reviewing,
        ClaimStatus::PossiblyEligible,
        ClaimStatus::ProofNeeded,
        ClaimStatus::Ready,
        ClaimStatus::OpenedOfficialForm,
        ClaimStatus::Submitted,
        ClaimStatus::Pending,
        ClaimStatus::Approved,
        ClaimStatus::Denied,
        ClaimStatus::Paid,
        ClaimStatus::Withdrawn,
        ClaimStatus::Expired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ClaimStatus::Discovered => "discovered",
            ClaimStatus::Reviewing => "reviewing",
            ClaimStatus::PossiblyEligible => "possibly_eligible",
            ClaimStatus::ProofNeeded => "proof_needed",
            ClaimStatus::Ready => "ready",
            ClaimStatus::OpenedOfficialForm => "opened_official_form",
            ClaimStatus::Submitted => "submitted",
            ClaimStatus::Pending => "pending",
            ClaimStatus::Approved => "approved",
            ClaimStatus::Denied => "denied",
            ClaimStatus::Paid => "paid",
            ClaimStatus::Withdrawn => "withdrawn",
            ClaimStatus::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.as_str() == s)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, ClaimStatus::Denied | ClaimStatus::Paid | ClaimStatus::Withdrawn | ClaimStatus::Expired)
    }

    /// Statuses that assert something happened outside OpenClaim.
    pub fn requires_user_confirmation(self) -> bool {
        matches!(
            self,
            ClaimStatus::Submitted | ClaimStatus::Pending | ClaimStatus::Approved | ClaimStatus::Denied | ClaimStatus::Paid
        )
    }

    fn rank(self) -> u8 {
        match self {
            ClaimStatus::Discovered => 0,
            ClaimStatus::Reviewing => 1,
            ClaimStatus::PossiblyEligible => 2,
            ClaimStatus::ProofNeeded => 3,
            ClaimStatus::Ready => 4,
            ClaimStatus::OpenedOfficialForm => 5,
            ClaimStatus::Submitted => 6,
            ClaimStatus::Pending => 7,
            ClaimStatus::Approved => 8,
            ClaimStatus::Denied | ClaimStatus::Paid | ClaimStatus::Withdrawn | ClaimStatus::Expired => 9,
        }
    }

    pub fn allowed_next(self) -> Vec<ClaimStatus> {
        use ClaimStatus::*;
        if self.is_terminal() {
            return Vec::new();
        }
        let mut next: Vec<ClaimStatus> = match self {
            // Pre-submission stages can move freely among themselves.
            Discovered | Reviewing | PossiblyEligible | ProofNeeded | Ready => {
                vec![Reviewing, PossiblyEligible, ProofNeeded, Ready, OpenedOfficialForm]
            }
            OpenedOfficialForm => vec![Ready, ProofNeeded, Submitted],
            Submitted => vec![Pending, Approved, Denied, Paid],
            Pending => vec![Approved, Denied, Paid],
            Approved => vec![Paid, Denied],
            Denied | Paid | Withdrawn | Expired => vec![],
        };
        next.retain(|s| *s != self);
        if self.rank() < Submitted.rank() {
            next.push(Expired);
        }
        next.push(Withdrawn);
        next
    }
}

/// Evidence accompanying a status change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TransitionEvidence {
    /// The user explicitly confirms the external event really happened.
    #[serde(default)]
    pub user_confirmed: bool,
    pub confirmation_id: Option<String>,
    pub document_id: Option<String>,
    pub note: Option<String>,
    /// Integer minor units (e.g. cents). Required for `paid`.
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
}

pub fn check_transition(from: ClaimStatus, to: ClaimStatus, ev: &TransitionEvidence) -> OcResult<()> {
    if !from.allowed_next().contains(&to) {
        return Err(
            OcError::conflict(format!("claim cannot move from '{}' to '{}'", from.as_str(), to.as_str())).with_details(serde_json::json!({
                "allowed": from.allowed_next().iter().map(|s| s.as_str()).collect::<Vec<_>>()
            })),
        );
    }
    if to.requires_user_confirmation() && !ev.user_confirmed {
        return Err(OcError::invalid(format!(
            "'{}' records an event at the official administrator; pass user_confirmed=true only if it really happened",
            to.as_str()
        )));
    }
    if to == ClaimStatus::Submitted && ev.confirmation_id.is_none() && ev.document_id.is_none() {
        return Err(OcError::invalid(
            "'submitted' needs submission evidence: an official confirmation id or a linked receipt document",
        ));
    }
    if matches!(to, ClaimStatus::Approved | ClaimStatus::Denied) && ev.document_id.is_none() && ev.note.as_deref().is_none_or(str::is_empty) {
        return Err(OcError::invalid(format!(
            "'{}' needs evidence: a linked notice document or a note describing the official notice",
            to.as_str()
        )));
    }
    if to == ClaimStatus::Paid {
        match (ev.amount_minor, ev.currency.as_deref()) {
            (Some(a), Some(c)) if a >= 0 && c.len() == 3 => {}
            _ => {
                return Err(OcError::invalid(
                    "'paid' needs amount_minor (integer minor units, >= 0) and a 3-letter currency code",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confirmed() -> TransitionEvidence {
        TransitionEvidence {
            user_confirmed: true,
            ..Default::default()
        }
    }

    #[test]
    fn cannot_skip_to_submitted() {
        assert!(check_transition(ClaimStatus::Discovered, ClaimStatus::Submitted, &confirmed()).is_err());
    }

    #[test]
    fn submission_requires_confirmation_and_evidence() {
        let from = ClaimStatus::OpenedOfficialForm;
        assert!(check_transition(from, ClaimStatus::Submitted, &TransitionEvidence::default()).is_err());
        assert!(check_transition(from, ClaimStatus::Submitted, &confirmed()).is_err());
        let ev = TransitionEvidence {
            confirmation_id: Some("ABC123".into()),
            ..confirmed()
        };
        assert!(check_transition(from, ClaimStatus::Submitted, &ev).is_ok());
    }

    #[test]
    fn paid_requires_integer_amount() {
        let ev = TransitionEvidence {
            amount_minor: Some(1250),
            currency: Some("USD".into()),
            ..confirmed()
        };
        assert!(check_transition(ClaimStatus::Approved, ClaimStatus::Paid, &ev).is_ok());
        assert!(check_transition(ClaimStatus::Approved, ClaimStatus::Paid, &confirmed()).is_err());
    }

    #[test]
    fn terminal_states_are_final() {
        assert!(ClaimStatus::Paid.allowed_next().is_empty());
        assert!(ClaimStatus::Discovered.allowed_next().contains(&ClaimStatus::Withdrawn));
        assert!(!ClaimStatus::Submitted.allowed_next().contains(&ClaimStatus::Expired));
    }
}
