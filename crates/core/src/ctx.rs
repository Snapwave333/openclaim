use crate::Service;
use openclaim_db::rusqlite::Connection;
use openclaim_domain::scopes::{PUBLIC_RESEARCH_SCOPES, scope_satisfies};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Credentials presented with a request. Never logged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    /// A human who proved the owner passphrase (CLI unlock or GUI login).
    Owner { session_id: String },
    /// A registered local principal (external agent or browser extension).
    Agent { id: String, name: String, scopes: Vec<String>, principal_kind: String },
    /// A local caller without credentials: public research only.
    Anonymous,
    /// The daemon's own scheduler and job runner.
    System,
}

impl Actor {
    pub fn id(&self) -> String {
        match self {
            Actor::Owner { .. } => "owner".into(),
            Actor::Agent { id, .. } => format!("agent:{id}"),
            Actor::Anonymous => "anonymous".into(),
            Actor::System => "system".into(),
        }
    }

    pub fn display(&self) -> String {
        match self {
            Actor::Owner { .. } => "owner".into(),
            Actor::Agent { name, .. } => format!("agent '{name}'"),
            Actor::Anonymous => "anonymous local caller".into(),
            Actor::System => "system".into(),
        }
    }

    pub fn is_owner(&self) -> bool {
        matches!(self, Actor::Owner { .. } | Actor::System)
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Actor::Agent { .. })
    }

    pub fn principal_id(&self) -> Option<&str> {
        match self {
            Actor::Agent { id, .. } => Some(id),
            _ => None,
        }
    }

    pub fn has_scope(&self, required: &str, resource: Option<&str>) -> bool {
        match self {
            Actor::Owner { .. } | Actor::System => true,
            Actor::Anonymous => PUBLIC_RESEARCH_SCOPES.contains(&required),
            Actor::Agent { scopes, .. } => scopes.iter().any(|g| scope_satisfies(g, required, resource)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub op: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub credentials: Credentials,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub non_interactive: bool,
    #[serde(default)]
    pub offline: bool,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub approval_id: Option<String>,
    /// Force the approval path even when the agent holds the scope (workflow approval nodes).
    #[serde(default)]
    pub require_approval: bool,
    #[serde(default = "default_transport")]
    pub transport: String,
}

fn default_transport() -> String {
    "ipc".into()
}

impl Request {
    pub fn new(op: &str, params: Value) -> Self {
        Request {
            op: op.into(),
            params,
            credentials: Credentials::default(),
            request_id: None,
            trace_id: None,
            dry_run: false,
            non_interactive: false,
            offline: false,
            idempotency_key: None,
            approval_id: None,
            require_approval: false,
            transport: "ipc".into(),
        }
    }

    pub fn with_session(mut self, s: &str) -> Self {
        self.credentials.session = Some(s.into());
        self
    }

    pub fn with_agent_token(mut self, t: &str) -> Self {
        self.credentials.agent_token = Some(t.into());
        self
    }
}

/// Per-operation context. `conn` is inside the request transaction.
pub struct Ctx<'a> {
    pub conn: &'a Connection,
    pub svc: &'a Service,
    pub actor: Actor,
    pub dry_run: bool,
    pub offline: bool,
    pub non_interactive: bool,
    pub now: String,
    pub request_id: String,
    pub trace_id: String,
    pub transport: String,
    pub warnings: Vec<String>,
    pub jobs_enqueued: bool,
}

impl Ctx<'_> {
    pub fn warn(&mut self, w: impl Into<String>) {
        self.warnings.push(w.into());
    }

    pub fn actor_id(&self) -> String {
        self.actor.id()
    }

    /// Records that personal data was shown to an agent principal.
    pub fn disclose(&self, subject_kind: &str, subject_id: &str, fields: &str) -> openclaim_domain::OcResult<()> {
        if let Some(pid) = self.actor.principal_id() {
            crate::sql::exec(
                self.conn,
                "INSERT INTO data_disclosures(id, principal_id, subject_kind, subject_id, fields, at) VALUES (?1,?2,?3,?4,?5,?6)",
                (openclaim_db::new_id(), pid, subject_kind, subject_id, fields, &self.now),
            )?;
        }
        Ok(())
    }

    pub fn audit(&self, action: &str, subject: &str, details: Value) -> openclaim_domain::OcResult<String> {
        crate::audit::append(self.conn, &self.actor_id(), action, subject, details)
    }

    pub fn emit(&self, topic: &str, payload: Value) -> openclaim_domain::OcResult<()> {
        crate::events::emit(self.conn, topic, payload).map(|_| ())
    }

    pub fn require_scope(&self, scope: &str, resource: Option<&str>) -> openclaim_domain::OcResult<()> {
        if self.actor.has_scope(scope, resource) {
            Ok(())
        } else {
            Err(openclaim_domain::OcError::denied(format!("{} lacks scope '{scope}'", self.actor.display())).with_scopes(vec![scope.to_string()]))
        }
    }
}
