//! Shared HTTP API contracts for the mobile/PWA clients.
//!
//! These are client-facing DTOs only: the server stays authoritative for authz,
//! scoring, clearance, and grounding. Client crates may depend on this module;
//! they must not depend on `presto-server` as code.

use serde::{Deserialize, Serialize};

pub type ActorId = String;
pub type SpaceId = String;
pub type DocumentId = String;
pub type SourceSectionId = String;

/// The JSON envelope used by Presto HTTP APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiEnvelope<T> {
    pub data: T,
}

/// Ordered confidentiality levels. A higher level may read lower levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidentialityLevel {
    Public,
    Internal,
    Confidential,
    Secret,
}

impl ConfidentialityLevel {
    pub fn allows(self, required: Self) -> bool {
        self >= required
    }
}

/// A user's role inside a space. Ordered like the SP-A membership model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceRole {
    Viewer,
    Contributor,
    Inviter,
    Admin,
    Owner,
}

/// Atomic capabilities the UI may render. The server still enforces every action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceCapability {
    Read,
    Contribute,
    AddDocument,
    Invite,
    ManageMembers,
    DeleteSpace,
}

impl SpaceRole {
    pub fn can(self, capability: SpaceCapability) -> bool {
        use SpaceCapability::{AddDocument, Contribute, DeleteSpace, Invite, ManageMembers, Read};
        match capability {
            Read => true,
            Contribute | AddDocument => self >= Self::Contributor,
            Invite => self >= Self::Inviter,
            ManageMembers => self >= Self::Admin,
            DeleteSpace => self >= Self::Owner,
        }
    }

    pub fn capabilities(self) -> Vec<SpaceCapability> {
        use SpaceCapability::{AddDocument, Contribute, DeleteSpace, Invite, ManageMembers, Read};
        [
            Read,
            Contribute,
            AddDocument,
            Invite,
            ManageMembers,
            DeleteSpace,
        ]
        .into_iter()
        .filter(|cap| self.can(*cap))
        .collect()
    }
}

/// Response for `GET /api/me`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentUser {
    /// Pseudonymous stable subject/actor id. Do not display it as a friendly name.
    pub actor_id: ActorId,
    /// Human-facing label for the current user. May be absent depending on IdP claims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The single-member space bootstrapped for the user's personal notebook.
    pub personal_space_id: SpaceId,
}

/// A compact space descriptor for mobile navigation and rights rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub id: SpaceId,
    pub name: String,
    pub role: SpaceRole,
    pub capabilities: Vec<SpaceCapability>,
    pub max_confidentiality: ConfidentialityLevel,
}

impl SpaceSummary {
    pub fn personal(id: impl Into<SpaceId>, name: impl Into<String>) -> Self {
        let role = SpaceRole::Owner;
        Self {
            id: id.into(),
            name: name.into(),
            role,
            capabilities: role.capabilities(),
            max_confidentiality: ConfidentialityLevel::Internal,
        }
    }
}

/// Response for `GET /api/spaces/current`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentSpace {
    pub space: SpaceSummary,
}

/// Server decision for an owner-uploaded document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentApprovalStatus {
    /// Exact bytes are not present in the independently approved registry.
    Pending,
    /// Exact bytes and hash match a pre-approved immutable fixture.
    Approved,
}

/// Bounded metadata returned by the owner corpus API. Document content is never
/// part of list or upload responses. `Pending` bodies/chunks are discarded, so
/// their `chunk_count` is always zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSummary {
    pub id: DocumentId,
    pub title: String,
    pub mime_type: String,
    pub byte_size: u32,
    pub chunk_count: u16,
    pub approval_status: DocumentApprovalStatus,
}

/// JSON upload accepted by `POST /api/corpus/documents`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentUploadRequest {
    pub filename: String,
    pub mime_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentUploadResult {
    pub document: DocumentSummary,
    pub deduplicated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentList {
    pub documents: Vec<DocumentSummary>,
}

/// Request body for `POST /api/rag/query`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RagQueryRequest {
    /// Target space. The server validates membership/capabilities; this is not trusted input.
    pub space_id: SpaceId,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sources: Option<u8>,
}

impl RagQueryRequest {
    pub fn new(space_id: impl Into<SpaceId>, query: impl Into<String>) -> Self {
        Self {
            space_id: space_id.into(),
            query: query.into(),
            max_sources: None,
        }
    }
}

/// A citation card the client may render without knowing server internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCitation {
    pub source_section_id: SourceSectionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_id: Option<DocumentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

/// Response body for `POST /api/rag/query`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RagQueryResponse {
    Grounded {
        answer: String,
        citations: Vec<SourceCitation>,
    },
    Rejected {
        /// Stable, user-safe reason code/message for verifier rejection.
        reason: String,
    },
}

impl RagQueryResponse {
    pub fn grounded(answer: impl Into<String>, citations: Vec<SourceCitation>) -> Self {
        Self::Grounded {
            answer: answer.into(),
            citations,
        }
    }

    pub fn rejected(reason: impl Into<String>) -> Self {
        Self::Rejected {
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_capabilities_are_ordered() {
        assert!(SpaceRole::Viewer.can(SpaceCapability::Read));
        assert!(!SpaceRole::Viewer.can(SpaceCapability::AddDocument));
        assert!(SpaceRole::Contributor.can(SpaceCapability::AddDocument));
        assert!(!SpaceRole::Inviter.can(SpaceCapability::ManageMembers));
        assert!(SpaceRole::Admin.can(SpaceCapability::ManageMembers));
        assert!(!SpaceRole::Admin.can(SpaceCapability::DeleteSpace));
        assert!(SpaceRole::Owner.can(SpaceCapability::DeleteSpace));
    }

    #[test]
    fn confidentiality_is_ordered() {
        assert!(ConfidentialityLevel::Secret.allows(ConfidentialityLevel::Public));
        assert!(ConfidentialityLevel::Confidential.allows(ConfidentialityLevel::Internal));
        assert!(!ConfidentialityLevel::Internal.allows(ConfidentialityLevel::Confidential));
    }

    #[test]
    fn rag_response_serializes_as_tagged_status() {
        let response = RagQueryResponse::grounded(
            "Paris is the capital of France.",
            vec![SourceCitation {
                source_section_id: "doc#p0".into(),
                document_id: Some("doc".into()),
                title: Some("Geography".into()),
                excerpt: None,
            }],
        );
        let json = serde_json::to_string(&ApiEnvelope { data: response }).unwrap();
        assert!(json.contains("\"status\":\"grounded\""));
        assert!(json.contains("\"source_section_id\":\"doc#p0\""));
    }

    #[test]
    fn personal_space_sets_owner_capabilities() {
        let space = SpaceSummary::personal("s1", "My notebook");
        assert_eq!(space.role, SpaceRole::Owner);
        assert!(space.capabilities.contains(&SpaceCapability::DeleteSpace));
    }

    #[test]
    fn document_contracts_are_closed_and_summaries_have_no_content() {
        assert!(serde_json::from_str::<DocumentUploadRequest>(
            r#"{"filename":"a.md","mime_type":"text/markdown","content":"x","space_id":"foreign"}"#
        )
        .is_err());
        let summary = DocumentSummary {
            id: "doc_1".into(),
            title: "a.md".into(),
            mime_type: "text/markdown".into(),
            byte_size: 1,
            chunk_count: 0,
            approval_status: DocumentApprovalStatus::Pending,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("content"));
        assert!(json.contains("pending"));
    }
}
