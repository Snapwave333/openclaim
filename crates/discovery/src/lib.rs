//! Source connectors. Every connector is declared by a manifest that must
//! state `price = "free"`; fetching respects robots rules, per-host rate
//! limits and an SSRF-safe allowlist. Parsers are pure functions over HTML so
//! they can run on fetched pages, user-saved snapshots and fixtures alike.

pub mod fetch;
pub mod manifest;
pub mod parse;
pub mod run;

pub use manifest::ConnectorManifest;
pub use parse::{EvidenceSpan, ParsedOpportunity, ParsedProgram, ParsedTip};
