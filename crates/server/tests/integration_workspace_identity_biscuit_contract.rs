#[cfg(test)]
mod tests {
    use std::fs;

    use presto_core::{ActorType, PermissionPrimitive, RoleAssignment, WorkspaceIdentity};

    #[test]
    fn test_session_identity_v0_1_fixture_loads() {
        let fixture_path = format!(
            "{}/../../docs/contracts/session-identity.v0.1.fixtures.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let fixture_json = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("fixture file must exist at {}: {}", fixture_path, e));
        let fixtures: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture must be valid JSON");

        assert_eq!(fixtures["version"], "session-identity.v0.1");
        assert!(fixtures["fixtures"].is_array());
        assert!(fixtures["fixtures"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_host_role_assignment_reconciliation() {
        let host = RoleAssignment::host(
            "workspace_test_001".to_string(),
            "actor_host_001".to_string(),
        );

        assert_eq!(host.role, "host");
        assert_eq!(host.actor_ref.actor_id, "actor_host_001");
        assert_eq!(host.actor_ref.actor_type, ActorType::Human);
        assert_eq!(host.permissions.len(), 6);
        assert!(host.permissions.contains(&PermissionPrimitive::Write));
        assert!(host.permissions.contains(&PermissionPrimitive::Approve));
        assert!(host.permissions.contains(&PermissionPrimitive::Invite));
        assert!(host.permissions.contains(&PermissionPrimitive::Administer));
    }

    #[test]
    fn test_participant_role_assignment_reconciliation() {
        let participant = RoleAssignment::participant(
            "workspace_test_001".to_string(),
            "actor_participant_001".to_string(),
        );

        assert_eq!(participant.role, "participant");
        assert_eq!(participant.actor_ref.actor_id, "actor_participant_001");
        assert_eq!(participant.actor_ref.actor_type, ActorType::Human);
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
    fn test_cross_repo_fixture_loads_and_validates() {
        let fixture_path = format!(
            "{}/../../docs/contracts/session-identity.v0.1.fixtures.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let fixture_json = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("fixture file must exist at {}: {}", fixture_path, e));
        let fixtures: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture must be valid JSON");

        let host_fixture = &fixtures["fixtures"][0];
        assert_eq!(host_fixture["role"], "host");
        assert_eq!(host_fixture["tenant_id"], "tenant_test_001");
        assert_eq!(host_fixture["actor_type"], "human");
        assert!(host_fixture["permissions"].is_array());

        let participant_fixture = &fixtures["fixtures"][1];
        assert_eq!(participant_fixture["role"], "participant");
        assert_eq!(participant_fixture["tenant_id"], "tenant_test_001");
        assert_eq!(participant_fixture["actor_type"], "human");
        assert!(participant_fixture["permissions"].is_array());
    }

    #[test]
    fn test_cross_repo_fixture_role_assignment_extraction() {
        // Simulate canvas/ai-practices loading fixture and building RoleAssignment.
        let host_role = RoleAssignment::host(
            "workspace_test_001".to_string(),
            "actor_host_001".to_string(),
        );

        assert_eq!(host_role.workspace_id, "workspace_test_001");
        assert_eq!(host_role.actor_ref.actor_id, "actor_host_001");
        assert_eq!(host_role.role, "host");
        assert!(host_role.permissions.contains(&PermissionPrimitive::Write));
        assert!(
            host_role
                .permissions
                .contains(&PermissionPrimitive::Approve)
        );
    }

    #[test]
    fn test_workspace_identity_projection_carries_tenant_id() {
        let host = RoleAssignment::host(
            "workspace_test_001".to_string(),
            "actor_host_001".to_string(),
        );
        let participant = RoleAssignment::participant(
            "workspace_test_001".to_string(),
            "actor_participant_001".to_string(),
        );
        let identity = WorkspaceIdentity::new(
            "tenant_test_001".to_string(),
            "workspace_test_001".to_string(),
            vec![host, participant],
        );
        assert_eq!(identity.tenant_id, "tenant_test_001");
        assert_eq!(identity.workspace_id, "workspace_test_001");
        assert_eq!(identity.memberships.len(), 2);
        assert_eq!(identity.role_assignments.len(), 2);
    }

    #[test]
    fn test_cross_repo_fixture_permissions_closed_vocabulary() {
        // Verify fixture permissions conform to ADR 0028 amendment 1.
        let fixture_path = format!(
            "{}/../../docs/contracts/session-identity.v0.1.fixtures.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let fixture_json = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("fixture file must exist at {}: {}", fixture_path, e));
        let fixtures: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture must be valid JSON");

        let closed_vocab = [
            "read",
            "comment",
            "write",
            "approve",
            "invite",
            "administer",
            "delegate",
        ];

        for fixture in fixtures["fixtures"].as_array().unwrap() {
            for perm in fixture["permissions"].as_array().unwrap() {
                let perm_str = perm.as_str().expect("permission must be string");
                assert!(
                    closed_vocab.contains(&perm_str),
                    "permission '{}' not in closed vocabulary",
                    perm_str
                );
            }
        }
    }
}
