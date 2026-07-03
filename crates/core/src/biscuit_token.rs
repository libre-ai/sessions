//! Biscuit token struct for session-identity.v0.1 contract.
//!
//! Represents a serialized Biscuit token with embedded metadata for
//! cross-repo fixture consumption (canvas, ai-practices).

use serde::{Deserialize, Serialize};

use crate::role_assignment::PermissionPrimitive;

/// Serialized Biscuit token with metadata.
/// Used in session-identity.v0.1.fixtures.json for cross-repo tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiscuitToken {
    /// The serialized token string (mock or real Biscuit).
    pub token_string: String,
    /// Workspace ID from token facts.
    pub workspace_id: String,
    /// Session ID from token facts.
    pub session_id: String,
    /// Actor ID from token facts.
    pub actor_id: String,
    /// Role: "host" or "participant".
    pub role: String,
    /// Permissions from closed vocabulary (ADR 0028 amendment 1).
    pub permissions: Vec<PermissionPrimitive>,
    /// Expiry timestamp (seconds since epoch).
    pub expiry_unix: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_biscuit_token_serializes() {
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
    }
}
