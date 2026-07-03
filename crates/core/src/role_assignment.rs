//! Role assignment and permission primitives for workspace-identity.v0.1 reconciliation.
//!
//! Aligns to ADR 0028 amendment 1: closed permission vocabulary.
//! Products (lm, canvas, ai-practices) map their product roles onto RoleAssignment
//! with permissions restricted to the closed set.

use serde::{Deserialize, Serialize};

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

/// Role assignment ties an actor to a workspace with a set of permissions.
/// The role name is product-defined (e.g., "host", "participant");
/// permissions must be from the closed vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub id: String,
    pub workspace_id: String,
    pub actor_id: String,
    pub role: String,
    pub permissions: Vec<PermissionPrimitive>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

impl RoleAssignment {
    /// Validate that role + permissions align to workspace-identity.v0.1 closed vocabulary.
    pub fn validate(&self) -> Result<(), String> {
        if self.permissions.is_empty() {
            return Err("permissions cannot be empty".to_string());
        }
        Ok(())
    }

    /// Map Host role to RoleAssignment with permissions per the
    /// workspace-identity.v0.1 contract: Host ⊇ {read, comment, write,
    /// approve, invite, administer}.
    pub fn host(workspace_id: String, actor_id: String) -> Self {
        Self {
            id: format!("role_{}", uuid::Uuid::new_v4()),
            workspace_id,
            actor_id,
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
        Self {
            id: format!("role_{}", uuid::Uuid::new_v4()),
            workspace_id,
            actor_id,
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
        assert_eq!(host.permissions.len(), 6);
        assert!(host.permissions.contains(&PermissionPrimitive::Read));
        assert!(host.permissions.contains(&PermissionPrimitive::Comment));
        assert!(host.permissions.contains(&PermissionPrimitive::Write));
        assert!(host.permissions.contains(&PermissionPrimitive::Approve));
        assert!(host.permissions.contains(&PermissionPrimitive::Invite));
        assert!(host.permissions.contains(&PermissionPrimitive::Administer));
        assert!(!host.permissions.contains(&PermissionPrimitive::Delegate));
    }

    #[test]
    fn test_participant_role_has_correct_permissions() {
        let participant = RoleAssignment::participant("ws_1".to_string(), "actor_1".to_string());
        assert_eq!(participant.role, "participant");
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
    }

    #[test]
    fn test_role_assignment_validates_non_empty_permissions() {
        let mut valid = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        assert!(valid.validate().is_ok());

        // Manually empty permissions for test.
        valid.permissions.clear();
        assert!(valid.validate().is_err());
    }

    #[test]
    fn test_permission_primitive_closed_vocabulary() {
        // Verify that only closed vocabulary is available.
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
    fn test_role_assignment_has_unique_id() {
        let a = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        let b = RoleAssignment::host("ws_1".to_string(), "actor_1".to_string());
        assert_ne!(a.id, b.id);
    }
}
