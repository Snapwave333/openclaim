//! The operation registry: one table of named, scoped, schema-described
//! operations. The CLI, MCP adapter, REST API and workflow runner all route
//! through it, so parity is structural rather than maintained by hand.

use crate::ctx::Ctx;
use openclaim_domain::OcResult;
use openclaim_domain::scopes::Effect;
use serde_json::Value;
use std::sync::LazyLock;

pub type Handler = fn(&mut Ctx<'_>, Value) -> OcResult<Value>;

pub struct OpSpec {
    pub name: &'static str,
    pub scope: &'static str,
    pub effect: Effect,
    pub summary: &'static str,
    pub params_schema: fn() -> Value,
    pub handler: Handler,
    /// Parameter that names the resource for narrowed scopes (`documents.read:<id>`).
    pub resource_param: Option<&'static str>,
    /// Exposed as an MCP tool under this name (None = CLI/REST only).
    pub mcp_tool: Option<&'static str>,
    /// Starts or performs network access (reported separately in dry runs).
    pub network: bool,
}

#[macro_export]
macro_rules! op {
    ($name:literal, $scope:literal, $effect:ident, $summary:literal, $params:ty, $handler:path $(, resource = $res:literal)? $(, mcp = $mcp:literal)? $(, network = $net:literal)?) => {
        $crate::ops::OpSpec {
            name: $name,
            scope: $scope,
            effect: openclaim_domain::scopes::Effect::$effect,
            summary: $summary,
            params_schema: || serde_json::to_value(schemars::schema_for!($params)).unwrap_or(serde_json::Value::Null),
            handler: $handler,
            resource_param: None $(.or(Some($res)))?,
            mcp_tool: None $(.or(Some($mcp)))?,
            network: false $(|| $net)?,
        }
    };
}

static REGISTRY: LazyLock<Vec<OpSpec>> = LazyLock::new(|| {
    let mut all: Vec<OpSpec> = Vec::new();
    all.extend(crate::modules::daemon::ops());
    all.extend(crate::modules::sources::ops());
    all.extend(crate::modules::opportunities::ops());
    all.extend(crate::modules::people::ops());
    all.extend(crate::modules::facts::ops());
    all.extend(crate::modules::documents::ops());
    all.extend(crate::modules::matching::ops());
    all.extend(crate::modules::claims::ops());
    all.extend(crate::modules::alerts::ops());
    all.extend(crate::modules::jobs_ops::ops());
    all.extend(crate::modules::workflows::ops());
    all.extend(crate::modules::agents::ops());
    all.extend(crate::modules::data::ops());
    all.extend(crate::modules::config::ops());
    all.extend(crate::modules::schema::ops());
    let mut seen = std::collections::HashSet::new();
    for op in &all {
        assert!(seen.insert(op.name), "duplicate op {}", op.name);
        assert!(openclaim_domain::scopes::is_known_scope(op.scope), "unknown scope {} on {}", op.scope, op.name);
    }
    all
});

pub fn registry() -> &'static [OpSpec] {
    &REGISTRY
}

pub fn find(name: &str) -> Option<&'static OpSpec> {
    REGISTRY.iter().find(|o| o.name == name)
}

/// Machine-readable description of every operation (for `schema export`, MCP and docs).
pub fn describe_all() -> Value {
    Value::Array(registry().iter().map(describe).collect())
}

pub fn describe(op: &OpSpec) -> Value {
    serde_json::json!({
        "name": op.name,
        "scope": op.scope,
        "effect": op.effect,
        "summary": op.summary,
        "network": op.network,
        "mcp_tool": op.mcp_tool,
        "resource_param": op.resource_param,
        "params_schema": (op.params_schema)(),
    })
}
