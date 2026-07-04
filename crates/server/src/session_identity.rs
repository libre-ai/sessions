//! Runtime bridge between live sessions and the shared workspace-identity.v0.1 contract.
//!
//! This is deliberately small: it gives the fixture-first session runtime a
//! tenant/workspace fact set without introducing an identity service, SSO/OIDC,
//! or a new persistence substrate. The cryptographic verifier still lives in
//! [`crate::auth`]; this module only derives the contract-facing scope and role
//! projection used by HTTP responses and token claims.

use presto_core::{ActorReference, ActorType, RoleAssignment, WorkspaceIdentity};
use serde::{Deserialize, Serialize};

/// Interim tenant boundary for the open wedge runtime.
///
/// A real external-identity source will replace this once a product needs it;
/// until then every token still carries an explicit tenant fact instead of
/// silently omitting the boundary required by workspace-identity.v0.1.
pub const DEFAULT_TENANT_ID: &str = "tenant_local";

/// Session-local workspace scope carried in runtime tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionScope {
    pub tenant_id: String,
    pub workspace_id: String,
    pub session_id: String,
}

impl SessionScope {
    /// Deterministic scope for the current open `/sessions` wedge.
    pub fn for_session(session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        Self {
            tenant_id: DEFAULT_TENANT_ID.to_string(),
            workspace_id: workspace_id_for_session(&session_id),
            session_id,
        }
    }

    pub fn try_new(
        tenant_id: impl Into<String>,
        workspace_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Result<Self, String> {
        let scope = Self {
            tenant_id: tenant_id.into(),
            workspace_id: workspace_id.into(),
            session_id: session_id.into(),
        };
        scope.validate()?;
        Ok(scope)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.tenant_id.trim().is_empty() {
            return Err("tenant_id cannot be empty".to_string());
        }
        if self.workspace_id.trim().is_empty() {
            return Err("workspace_id cannot be empty".to_string());
        }
        if self.session_id.trim().is_empty() {
            return Err("session_id cannot be empty".to_string());
        }
        Ok(())
    }
}

fn workspace_id_for_session(session_id: &str) -> String {
    format!("workspace_{session_id}")
}

/// Build the accepted contract role projection for an LM runtime actor.
pub fn role_assignment_for_actor(
    scope: &SessionScope,
    actor_id: impl Into<String>,
    role: SessionRole,
) -> RoleAssignment {
    let actor_ref = ActorReference {
        actor_id: actor_id.into(),
        actor_type: ActorType::Human,
        display_name: None,
        source: Some("session_identity_runtime".to_string()),
    };
    match role {
        SessionRole::Host => RoleAssignment::host_for_actor(scope.workspace_id.clone(), actor_ref),
        SessionRole::Participant => {
            RoleAssignment::participant_for_actor(scope.workspace_id.clone(), actor_ref)
        }
    }
}

/// Product roles understood by the LM runtime token bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Host,
    Participant,
}

impl SessionRole {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionRole::Host => "host",
            SessionRole::Participant => "participant",
        }
    }
}

/// Build a minimal `WorkspaceIdentity` fact set for a session actor.
pub fn workspace_identity_for_actor(
    scope: &SessionScope,
    actor_id: impl Into<String>,
    role: SessionRole,
) -> WorkspaceIdentity {
    let assignment = role_assignment_for_actor(scope, actor_id, role);
    WorkspaceIdentity::new(
        scope.tenant_id.clone(),
        scope.workspace_id.clone(),
        vec![assignment],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_scope_carries_required_tenant_and_workspace() {
        let scope = SessionScope::for_session("ABC123");
        assert_eq!(scope.tenant_id, DEFAULT_TENANT_ID);
        assert_eq!(scope.workspace_id, "workspace_ABC123");
        assert_eq!(scope.session_id, "ABC123");
        assert!(scope.validate().is_ok());
    }

    #[test]
    fn workspace_identity_projection_uses_contract_shape() {
        let scope = SessionScope::for_session("S1");
        let identity = workspace_identity_for_actor(&scope, "host-1", SessionRole::Host);
        let json = serde_json::to_value(identity).expect("identity serializes");
        assert_eq!(json["tenant_id"], DEFAULT_TENANT_ID);
        assert_eq!(json["workspace_id"], "workspace_S1");
        assert_eq!(
            json["role_assignments"][0]["actor_ref"]["actor_type"],
            "human"
        );
        assert_eq!(json["role_assignments"][0]["role"], "host");
        assert!(json["role_assignments"][0].get("actor_id").is_none());
    }

    #[test]
    fn empty_scope_parts_are_rejected() {
        assert!(SessionScope::try_new("", "workspace", "session").is_err());
        assert!(SessionScope::try_new("tenant", "", "session").is_err());
        assert!(SessionScope::try_new("tenant", "workspace", "").is_err());
    }
}
