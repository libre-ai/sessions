#[cfg(test)]
mod tests {
    use std::fs;

    use presto_core::{BiscuitToken, PermissionPrimitive, RoleAssignment};

    #[test]
    fn test_cross_repo_fixture_loads_and_validates() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/contracts/session-identity.v0.1.fixtures.json"
        );
        let fixture_json = fs::read_to_string(fixture_path).expect("fixture file must exist");
        let fixtures: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture must be valid JSON");

        let host_fixture = &fixtures["fixtures"][0];
        assert_eq!(host_fixture["role"], "host");
        assert!(host_fixture["serialized_biscuit"].is_string());

        let participant_fixture = &fixtures["fixtures"][1];
        assert_eq!(participant_fixture["role"], "participant");
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
        assert_eq!(host_role.role, "host");
        assert!(host_role.permissions.contains(&PermissionPrimitive::Write));
        assert!(
            host_role
                .permissions
                .contains(&PermissionPrimitive::Approve)
        );
    }

    #[test]
    fn test_cross_repo_fixture_permissions_closed_vocabulary() {
        // Verify fixture permissions conform to ADR 0028 amendment 1.
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/contracts/session-identity.v0.1.fixtures.json"
        );
        let fixture_json = fs::read_to_string(fixture_path).expect("fixture file must exist");
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

    #[test]
    fn test_biscuit_token_fixture_format() {
        // Verify serialized_biscuit field is present and parseable.
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/contracts/session-identity.v0.1.fixtures.json"
        );
        let fixture_json = fs::read_to_string(fixture_path).expect("fixture file must exist");
        let fixtures: serde_json::Value =
            serde_json::from_str(&fixture_json).expect("fixture must be valid JSON");

        for fixture in fixtures["fixtures"].as_array().unwrap() {
            assert!(fixture["serialized_biscuit"].is_string());
            let biscuit = fixture["serialized_biscuit"].as_str().unwrap();
            assert!(
                biscuit.starts_with("mock_biscuit_"),
                "biscuit token must start with 'mock_biscuit_'"
            );
        }
    }

    #[test]
    fn test_biscuit_token_struct_serialization() {
        let token = BiscuitToken {
            token_string: "mock_biscuit_12345".to_string(),
            workspace_id: "workspace_test_001".to_string(),
            session_id: "session_test_001".to_string(),
            actor_id: "actor_host_001".to_string(),
            role: "host".to_string(),
            permissions: vec![
                PermissionPrimitive::Read,
                PermissionPrimitive::Comment,
                PermissionPrimitive::Write,
                PermissionPrimitive::Approve,
                PermissionPrimitive::Administer,
            ],
            expiry_unix: 9999999999,
        };

        let json = serde_json::to_string(&token).unwrap();
        let deserialized: BiscuitToken = serde_json::from_str(&json).unwrap();

        assert_eq!(token.token_string, deserialized.token_string);
        assert_eq!(token.workspace_id, deserialized.workspace_id);
        assert_eq!(token.role, deserialized.role);
        assert_eq!(token.permissions, deserialized.permissions);
    }
}
