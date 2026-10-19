use crate::error::OcError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: &str = "1.1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Pagination {
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Snapshot marker so pages are not mixed across refreshes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

/// The single machine-readable envelope written to stdout in `--output json`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Envelope {
    pub schema_version: String,
    pub ok: bool,
    pub request_id: String,
    pub trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OcError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
}

impl Envelope {
    pub fn success(request_id: String, trace_id: String, data: Value) -> Self {
        Envelope {
            schema_version: SCHEMA_VERSION.into(),
            ok: true,
            request_id,
            trace_id,
            data: Some(data),
            error: None,
            warnings: Vec::new(),
            pagination: None,
            audit_event_id: None,
            dry_run: false,
        }
    }

    pub fn failure(request_id: String, trace_id: String, error: OcError) -> Self {
        Envelope {
            schema_version: SCHEMA_VERSION.into(),
            ok: false,
            request_id,
            trace_id,
            data: None,
            error: Some(error),
            warnings: Vec::new(),
            pagination: None,
            audit_event_id: None,
            dry_run: false,
        }
    }

    pub fn exit_code(&self) -> i32 {
        match &self.error {
            None => 0,
            Some(e) => e.code.exit_code(),
        }
    }
}
