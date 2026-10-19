//! Domain types shared by every OpenClaim surface (daemon, CLI, MCP, REST, GUI).
//!
//! Nothing in this crate performs I/O. Eligibility is evaluated with a
//! deterministic three-valued rules engine; there is no model inference anywhere.

pub mod claims;
pub mod dates;
pub mod envelope;
pub mod error;
pub mod model;
pub mod rules;
pub mod scopes;

pub use envelope::{Envelope, SCHEMA_VERSION};
pub use error::{ErrorCode, OcError, OcResult};

pub const PRODUCT_NAME: &str = "OpenClaim";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
