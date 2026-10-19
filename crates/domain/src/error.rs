use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Public error classes. Exit codes are a compatibility guarantee (MDR §22.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    NotFound,
    PermissionDenied,
    ApprovalRequired,
    Conflict,
    Unavailable,
    Timeout,
    PolicyBlocked,
    PartialResult,
    Internal,
}

impl ErrorCode {
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCode::InvalidArgument => 2,
            ErrorCode::NotFound => 3,
            ErrorCode::PermissionDenied => 4,
            ErrorCode::ApprovalRequired => 5,
            ErrorCode::Conflict => 6,
            ErrorCode::Unavailable => 7,
            ErrorCode::Timeout => 8,
            ErrorCode::PolicyBlocked => 9,
            ErrorCode::PartialResult => 10,
            ErrorCode::Internal => 11,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ErrorCode::ApprovalRequired => "APPROVAL_REQUIRED",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::Unavailable => "UNAVAILABLE",
            ErrorCode::Timeout => "TIMEOUT",
            ErrorCode::PolicyBlocked => "POLICY_BLOCKED",
            ErrorCode::PartialResult => "PARTIAL_RESULT",
            ErrorCode::Internal => "INTERNAL",
        }
    }

    pub fn default_retryable(self) -> bool {
        matches!(self, ErrorCode::Unavailable | ErrorCode::Timeout | ErrorCode::Conflict)
    }

    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::InvalidArgument => 400,
            ErrorCode::NotFound => 404,
            ErrorCode::PermissionDenied => 403,
            ErrorCode::ApprovalRequired => 428,
            ErrorCode::Conflict => 409,
            ErrorCode::Unavailable => 503,
            ErrorCode::Timeout => 504,
            ErrorCode::PolicyBlocked => 451,
            ErrorCode::PartialResult => 206,
            ErrorCode::Internal => 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct OcError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub needed_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

pub type OcResult<T> = Result<T, OcError>;

impl OcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        OcError {
            code,
            message: message.into(),
            retryable: code.default_retryable(),
            retry_after: None,
            approval_id: None,
            needed_scopes: Vec::new(),
            next_action: None,
            source_status: None,
            details: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }
    pub fn denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PermissionDenied, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }
    pub fn policy(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PolicyBlocked, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn with_next_action(mut self, next: impl Into<String>) -> Self {
        self.next_action = Some(next.into());
        self
    }
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.needed_scopes = scopes;
        self
    }
}

impl From<serde_json::Error> for OcError {
    fn from(e: serde_json::Error) -> Self {
        OcError::invalid(format!("invalid JSON: {e}"))
    }
}
