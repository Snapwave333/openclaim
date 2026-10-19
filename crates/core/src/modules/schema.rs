//! Machine-discoverable contracts: envelope, error codes, every operation's schema.

use super::parse_params;
use crate::ctx::Ctx;
use crate::op;
use crate::ops::{self, OpSpec};
use openclaim_domain::{ErrorCode, OcError, OcResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn error_codes() -> Value {
    let all = [ErrorCode::InvalidArgument, ErrorCode::NotFound, ErrorCode::PermissionDenied, ErrorCode::ApprovalRequired, ErrorCode::Conflict, ErrorCode::Unavailable, ErrorCode::Timeout, ErrorCode::PolicyBlocked, ErrorCode::PartialResult, ErrorCode::Internal];
    json!(all.iter().map(|c| json!({"code": c.as_str(), "exit_code": c.exit_code(), "http_status": c.http_status(), "retryable_by_default": c.default_retryable()})).collect::<Vec<_>>())
}

pub fn full_export() -> Value {
    json!({
        "product": openclaim_domain::PRODUCT_NAME,
        "version": openclaim_domain::VERSION,
        "schema_version": openclaim_domain::SCHEMA_VERSION,
        "envelope": serde_json::to_value(schemars::schema_for!(openclaim_domain::Envelope)).unwrap_or(Value::Null),
        "error": serde_json::to_value(schemars::schema_for!(openclaim_domain::OcError)).unwrap_or(Value::Null),
        "error_codes": error_codes(),
        "scopes": openclaim_domain::scopes::ALL_SCOPES.iter().map(|(s, d)| json!({"scope": s, "description": d})).collect::<Vec<_>>(),
        "global_flags": ["--profile", "--output", "--fields", "--limit", "--cursor", "--timeout", "--offline", "--dry-run", "--request-id", "--idempotency-key", "--non-interactive", "--approval"],
        "operations": ops::describe_all(),
        "rules_ast": serde_json::to_value(schemars::schema_for!(openclaim_domain::rules::RuleSet)).unwrap_or(Value::Null),
        "claim_statuses": openclaim_domain::claims::ClaimStatus::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "match_states": ["potential_match", "needs_more_information", "unlikely", "not_applicable", "manual_review"],
    })
}

fn op_export(_ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(full_export())
}

fn op_commands(_ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({"commands": ops::registry().iter().map(|o| json!({"name": o.name, "scope": o.scope, "effect": o.effect, "summary": o.summary, "mcp_tool": o.mcp_tool})).collect::<Vec<_>>()}))
}

#[derive(Deserialize, JsonSchema)]
pub struct OpParam {
    pub op: String,
}

fn op_describe(_ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: OpParam = parse_params(p)?;
    let op = ops::find(&p.op).ok_or_else(|| OcError::not_found(format!("operation '{}' not found", p.op)))?;
    Ok(ops::describe(op))
}

fn op_version(_ctx: &mut Ctx<'_>, _p: Value) -> OcResult<Value> {
    Ok(json!({"version": openclaim_domain::VERSION, "schema_version": openclaim_domain::SCHEMA_VERSION, "operations": ops::registry().len()}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ValidateParams {
    pub op: String,
    #[serde(default)]
    pub params: Value,
}

/// Structural validation against the operation's schema: required keys, unknown keys, basic types.
pub fn validate_params(spec: &OpSpec, params: &Value) -> Vec<String> {
    let schema = (spec.params_schema)();
    let mut problems = Vec::new();
    let Some(obj) = params.as_object() else {
        if !params.is_null() {
            problems.push("params must be an object".into());
        }
        return problems;
    };
    if let Some(req) = schema["required"].as_array() {
        for r in req {
            if let Some(k) = r.as_str() {
                if !obj.contains_key(k) {
                    problems.push(format!("missing required parameter '{k}'"));
                }
            }
        }
    }
    if let Some(props) = schema["properties"].as_object() {
        let additional = schema["additionalProperties"].as_bool().unwrap_or(true);
        for (k, v) in obj {
            match props.get(k) {
                None if !additional => problems.push(format!("unknown parameter '{k}'")),
                None => {}
                Some(p) => {
                    let ty = p["type"].as_str().or_else(|| p["type"].as_array().and_then(|a| a.first()).and_then(|t| t.as_str()));
                    let ok = match ty {
                        Some("string") => v.is_string() || v.is_null(),
                        Some("boolean") => v.is_boolean() || v.is_null(),
                        Some("integer") | Some("number") => v.is_number() || v.is_null(),
                        Some("array") => v.is_array() || v.is_null(),
                        Some("object") => v.is_object() || v.is_null(),
                        _ => true,
                    };
                    if !ok {
                        problems.push(format!("parameter '{k}' should be {}", ty.unwrap_or("?")));
                    }
                }
            }
        }
    }
    problems
}

fn op_validate(_ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ValidateParams = parse_params(p)?;
    let op = ops::find(&p.op).ok_or_else(|| OcError::not_found(format!("operation '{}' not found", p.op)))?;
    let problems = validate_params(op, &p.params);
    Ok(json!({"op": p.op, "valid": problems.is_empty(), "problems": problems}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("schema.export", "schema.read", Read, "Full machine-readable contract: envelope, errors, scopes, operations", super::Empty, op_export),
        op!("schema.commands", "schema.read", Read, "Operation names, scopes, effects and summaries", super::Empty, op_commands),
        op!("schema.describe", "schema.read", Read, "One operation's parameter schema", OpParam, op_describe),
        op!("schema.version", "schema.read", Read, "Product and contract versions", super::Empty, op_version),
        op!("schema.validate", "schema.read", Read, "Validate parameters against an operation's schema", ValidateParams, op_validate),
    ]
}
