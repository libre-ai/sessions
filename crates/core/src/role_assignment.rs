//! Role assignment and permission primitives for workspace-identity.v0.1 reconciliation.
//!
//! Aligns to ADR 0028 amendment 1: closed permission vocabulary.
//! Products (lm, canvas, ai-practices) map their product roles onto RoleAssignment
//! with permissions restricted to the closed set.

use serde::{Deserialize, Serialize};

/// Actor type from workspace-identity.v0.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    Human,
    Agent,
    Service,
    External,
}

/// Actor reference from workspace-identity.v0.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct ActorReference {
    pub actor_id: String,
    pub actor_type: ActorType,
    pub display_name: Option<String>,
    pub source: Option<String>,
}

impl ActorReference {
    pub fn human(actor_id: String) -> Self {
        Self {
            actor_id,
            actor_type: ActorType::Human,
            display_name: None,
            source: Some("session_identity".to_string()),
        }
    }
}

/// Permission primitives (closed vocabulary, v0.1).
/// Per ADR 0028 amendment 1: these are the only allowed permissions.
/// Adding a new primitive requires a v0.2 contract change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPrimitive {
    Read,
    Comment,
    Write,
    Approve,
    Invite,
    Administer,
    Delegate,
}

/// Membership status from workspace-identity.v0.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MembershipStatus {
    Active,
    Invited,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMembership {
    pub id: String,
    pub workspace_id: String,
    pub actor_ref: ActorReference,
    pub status: MembershipStatus,
    pub joined_at: String,
    pub revoked_at: Option<String>,
}

/// Role assignment ties an actor to a workspace with a set of permissions.
/// The role name is product-defined (e.g., "host", "participant");
/// permissions must be from the closed vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub id: String,
    pub workspace_id: String,
    pub actor_ref: ActorReference,
    pub role: String,
    pub permissions: Vec<PermissionPrimitive>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

/// Minimal WorkspaceIdentity fact set for LM fixtures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub workspace_id: String,
    pub tenant_id: String,
    pub memberships: Vec<WorkspaceMembership>,
    pub role_assignments: Vec<RoleAssignment>,
}

impl WorkspaceIdentity {
    pub fn new(tenant_id: String, workspace_id: String, roles: Vec<RoleAssignment>) -> Self {
        Self::try_new(tenant_id, workspace_id, roles)
            .expect("workspace identity roles must match the tenant/workspace boundary")
    }

    pub fn try_new(
        tenant_id: String,
        workspace_id: String,
        roles: Vec<RoleAssignment>,
    ) -> Result<Self, String> {
        if tenant_id.trim().is_empty() {
            return Err("tenant_id cannot be empty".to_string());
        }
        if workspace_id.trim().is_empty() {
            return Err("workspace_id cannot be empty".to_string());
        }
        for role in &roles {
            role.validate()?;
            if role.workspace_id != workspace_id {
                return Err("role workspace_id must match WorkspaceIdentity root".to_string());
            }
        }

        let memberships = roles
            .iter()
            .map(|role| WorkspaceMembership {
                id: format!("membership_{}", role.id),
                workspace_id: workspace_id.clone(),
                actor_ref: role.actor_ref.clone(),
                status: MembershipStatus::Active,
                joined_at: role.created_at.clone(),
                revoked_at: None,
            })
            .collect();

        Ok(Self {
            workspace_id,
            tenant_id,
            memberships,
            role_assignments: roles,
        })
    }
}

impl RoleAssignment {
    /// Validate that role + permissions align to workspace-identity.v0.1 closed vocabulary.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("id cannot be empty".to_string());
        }
        if self.workspace_id.trim().is_empty() {
            return Err("workspace_id cannot be empty".to_string());
        }
        if self.actor_ref.actor_id.trim().is_empty() {
            return Err("actor_id cannot be empty".to_string());
        }
        if self.role.trim().is_empty() {
            return Err("role cannot be empty".to_string());
        }
        if self.permissions.is_empty() {
            return Err("permissions cannot be empty".to_string());
        }
        if matches!(
            self.actor_ref.actor_type,
            ActorType::Agent | ActorType::Service | ActorType::External
        ) && self.permissions.iter().any(|permission| {
            matches!(
                permission,
                PermissionPrimitive::Approve | PermissionPrimitive::Delegate
            )
        }) {
            return Err("non-human actors cannot hold approve or delegate".to_string());
        }
        Ok(())
    }

    /// Map Host role to RoleAssignment with permissions per the
    /// workspace-identity.v0.1 contract: Host ⊇ {read, comment, write,
    /// approve, invite, administer}.
    pub fn host(workspace_id: String, actor_id: String) -> Self {
        Self::host_for_actor(workspace_id, ActorReference::human(actor_id))
    }

    pub fn host_for_actor(workspace_id: String, actor_ref: ActorReference) -> Self {
        Self {
            id: format!("role_{}", uuid::Uuid::new_v4()),
            workspace_id,
            actor_ref,
            role: "host".to_string(),
            permissions: vec![
                PermissionPrimitive::Read,
                PermissionPrimitive::Comment,
                PermissionPrimitive::Write,
                PermissionPrimitive::Approve,
                PermissionPrimitive::Invite,
                PermissionPrimitive::Administer,
            ],
            created_at: chrono::Utc::now().to_rfc3339(),
            revoked_at: None,
        }
    }

    /// Map Participant role to RoleAssignment with permissions per the
    /// workspace-identity.v0.1 contract: Participant ⊇ {read, comment, write}
    /// (write = submitting answers).
    pub fn participant(workspace_id: String, actor_id: String) -> Self {
        Self::participant_for_actor(workspace_id, ActorReference::human(actor_id))
    }

    pub fn participant_for_actor(workspace_id: String, actor_ref: ActorReference) -> Self {
        Self {
            id: format!("role_{}", uuid::Uuid::new_v4()),
            workspace_id,
            actor_ref,
            role: "participant".to_string(),
            permissions: vec![
                PermissionPrimitive::Read,
                PermissionPrimitive::Comment,
                PermissionPrimitive::Write,
            ],
            created_at: chrono::Utc::now().to_rfc3339(),
            revoked_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_role_has_correct_permissions() {
        let host = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        assert_eq!(host.role, "host");
        assert_eq!(host.actor_ref.actor_id, "actor_1");
        assert_eq!(host.actor_ref.actor_type, ActorType::Human);
        assert_eq!(host.permissions.len(), 6);
        assert!(host.permissions.contains(&PermissionPrimitive::Read));
        assert!(host.permissions.contains(&PermissionPrimitive::Comment));
        assert!(host.permissions.contains(&PermissionPrimitive::Write));
        assert!(host.permissions.contains(&PermissionPrimitive::Approve));
        assert!(host.permissions.contains(&PermissionPrimitive::Invite));
        assert!(host.permissions.contains(&PermissionPrimitive::Administer));
        assert!(!host.permissions.contains(&PermissionPrimitive::Delegate));
        assert!(host.validate().is_ok());
    }

    #[test]
    fn test_participant_role_has_correct_permissions() {
        let participant = RoleAssignment::participant("ws_1".to_string(), "actor_1".to_string());
        assert_eq!(participant.role, "participant");
        assert_eq!(participant.actor_ref.actor_id, "actor_1");
        assert_eq!(participant.permissions.len(), 3);
        assert!(participant.permissions.contains(&PermissionPrimitive::Read));
        assert!(
            participant
                .permissions
                .contains(&PermissionPrimitive::Comment)
        );
        assert!(
            participant
                .permissions
                .contains(&PermissionPrimitive::Write)
        );
        assert!(
            !participant
                .permissions
                .contains(&PermissionPrimitive::Approve)
        );
        assert!(
            !participant
                .permissions
                .contains(&PermissionPrimitive::Administer)
        );
        assert!(participant.validate().is_ok());
    }

    #[test]
    fn test_role_assignment_validates_non_empty_permissions() {
        let mut valid = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        assert!(valid.validate().is_ok());

        valid.permissions.clear();
        assert!(valid.validate().is_err());
    }

    #[test]
    fn test_non_human_actor_cannot_hold_approval_permission() {
        let role = RoleAssignment::host_for_actor(
            "ws_1".to_string(),
            ActorReference {
                actor_id: "svc_1".to_string(),
                actor_type: ActorType::Service,
                display_name: None,
                source: Some("fixture".to_string()),
            },
        );
        assert!(role.validate().is_err());
    }

    #[test]
    fn test_agent_participant_without_approval_is_valid() {
        let role = RoleAssignment::participant_for_actor(
            "ws_1".to_string(),
            ActorReference {
                actor_id: "agent_1".to_string(),
                actor_type: ActorType::Agent,
                display_name: None,
                source: Some("fixture".to_string()),
            },
        );
        assert!(role.validate().is_ok());
    }

    #[test]
    fn test_actor_type_serializes_as_contract_snake_case() {
        let actor = ActorReference {
            actor_id: "actor_1".to_string(),
            actor_type: ActorType::Human,
            display_name: None,
            source: None,
        };
        let value = serde_json::to_value(actor).expect("actor serializes");
        assert_eq!(value["actor_type"], "human");
    }

    #[test]
    fn test_role_assignment_serializes_actor_ref_not_actor_id() {
        let role = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        let value = serde_json::to_value(role).expect("role assignment serializes");
        assert!(value.get("actor_ref").is_some());
        assert!(value.get("actor_id").is_none());
        assert_eq!(value["actor_ref"]["actor_id"], "actor_1");
    }

    #[test]
    fn test_permission_primitive_closed_vocabulary() {
        let perms = [
            PermissionPrimitive::Read,
            PermissionPrimitive::Comment,
            PermissionPrimitive::Write,
            PermissionPrimitive::Approve,
            PermissionPrimitive::Invite,
            PermissionPrimitive::Administer,
            PermissionPrimitive::Delegate,
        ];
        assert_eq!(
            perms.len(),
            7,
            "exactly 7 closed permissions per ADR 0028 amend#1"
        );
    }

    #[test]
    fn test_workspace_identity_root_carries_tenant_id() {
        let host = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        let identity =
            WorkspaceIdentity::new("tenant_1".to_string(), "ws_1".to_string(), vec![host]);
        let value = serde_json::to_value(identity).expect("workspace identity serializes");
        assert_eq!(value["tenant_id"], "tenant_1");
        assert_eq!(value["workspace_id"], "ws_1");
        assert!(
            value["memberships"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            value["role_assignments"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
    }

    #[test]
    fn test_workspace_identity_rejects_cross_workspace_role() {
        let host = RoleAssignment::host("ws_other".to_string(), "actor_1".to_string());
        let result =
            WorkspaceIdentity::try_new("tenant_1".to_string(), "ws_1".to_string(), vec![host]);
        assert!(result.is_err());
    }

    #[test]
    fn test_role_assignment_has_unique_id() {
        let a = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        let b = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        assert_ne!(a.id, b.id);
    }
}
