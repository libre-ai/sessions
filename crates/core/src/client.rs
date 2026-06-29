//! Pure client-side state machines shared by mobile/PWA clients.
//!
//! These types are intentionally UI-framework agnostic. Dioxus components render
//! them, but state transitions that carry product meaning live here in Rust.

use serde::{Deserialize, Serialize};

use crate::api::{CurrentUser, RagQueryResponse, SourceCitation};

/// Authentication/session state as seen by the mobile client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AuthSessionState {
    Unknown,
    Anonymous,
    Authenticated { user: CurrentUser },
    Expired { reason: String },
}

impl AuthSessionState {
    pub fn authenticated(user: CurrentUser) -> Self {
        Self::Authenticated { user }
    }

    pub fn is_authenticated(&self) -> bool {
        matches!(self, Self::Authenticated { .. })
    }

    pub fn logout(&mut self) {
        *self = Self::Anonymous;
    }

    pub fn expire(&mut self, reason: impl Into<String>) {
        *self = Self::Expired {
            reason: reason.into(),
        };
    }
}

/// A bounded user-input error. Keep this user-safe: it may be rendered directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientInputError {
    EmptyQuery,
}

impl std::fmt::Display for ClientInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyQuery => write!(f, "query is required"),
        }
    }
}

impl std::error::Error for ClientInputError {}

/// Mobile RAG query lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RagQueryState {
    Idle,
    Draft {
        query: String,
    },
    Loading {
        query: String,
    },
    Grounded {
        query: String,
        answer: String,
        citations: Vec<SourceCitation>,
    },
    Rejected {
        query: String,
        reason: String,
    },
    Failed {
        query: String,
        message: String,
    },
}

impl RagQueryState {
    pub fn edit(query: impl Into<String>) -> Self {
        Self::Draft {
            query: query.into(),
        }
    }

    pub fn submit(query: impl AsRef<str>) -> Result<Self, ClientInputError> {
        let query = query.as_ref().trim().to_string();
        if query.is_empty() {
            return Err(ClientInputError::EmptyQuery);
        }
        Ok(Self::Loading { query })
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }

    pub fn query(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Draft { query }
            | Self::Loading { query }
            | Self::Grounded { query, .. }
            | Self::Rejected { query, .. }
            | Self::Failed { query, .. } => Some(query),
        }
    }

    pub fn apply_response(self, response: RagQueryResponse) -> Self {
        let query = self.query().unwrap_or_default().to_string();
        match response {
            RagQueryResponse::Grounded { answer, citations } => Self::Grounded {
                query,
                answer,
                citations,
            },
            RagQueryResponse::Rejected { reason } => Self::Rejected { query, reason },
        }
    }

    pub fn fail(self, message: impl Into<String>) -> Self {
        Self::Failed {
            query: self.query().unwrap_or_default().to_string(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CurrentUser, RagQueryResponse, SourceCitation};

    fn user() -> CurrentUser {
        CurrentUser {
            actor_id: "sub-123".into(),
            display_name: Some("Ada".into()),
            personal_space_id: "space-1".into(),
        }
    }

    #[test]
    fn auth_state_transitions_do_not_expose_tokens() {
        let mut state = AuthSessionState::authenticated(user());
        assert!(state.is_authenticated());

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("authenticated"));
        assert!(!json.contains("token"));

        state.expire("session_expired");
        assert_eq!(
            state,
            AuthSessionState::Expired {
                reason: "session_expired".into()
            }
        );
        state.logout();
        assert_eq!(state, AuthSessionState::Anonymous);
    }

    #[test]
    fn rag_submit_trims_and_rejects_empty_queries() {
        assert_eq!(
            RagQueryState::submit("   "),
            Err(ClientInputError::EmptyQuery)
        );
        assert_eq!(
            RagQueryState::submit("  capital of France  ").unwrap(),
            RagQueryState::Loading {
                query: "capital of France".into()
            }
        );
    }

    #[test]
    fn rag_loading_resolves_to_grounded_answer() {
        let state = RagQueryState::submit("capital").unwrap();
        let resolved = state.apply_response(RagQueryResponse::grounded(
            "Paris.",
            vec![SourceCitation {
                source_section_id: "geo#fr".into(),
                document_id: None,
                title: None,
                excerpt: Some("Paris is the capital of France.".into()),
            }],
        ));
        assert_eq!(
            resolved,
            RagQueryState::Grounded {
                query: "capital".into(),
                answer: "Paris.".into(),
                citations: vec![SourceCitation {
                    source_section_id: "geo#fr".into(),
                    document_id: None,
                    title: None,
                    excerpt: Some("Paris is the capital of France.".into()),
                }]
            }
        );
    }

    #[test]
    fn verifier_rejection_is_a_distinct_state() {
        let state = RagQueryState::submit("unsupported claim").unwrap();
        let rejected = state.apply_response(RagQueryResponse::rejected("generation_ungrounded"));
        assert_eq!(
            rejected,
            RagQueryState::Rejected {
                query: "unsupported claim".into(),
                reason: "generation_ungrounded".into(),
            }
        );
    }

    #[test]
    fn backend_failure_keeps_query_context() {
        let state = RagQueryState::submit("capital").unwrap();
        assert_eq!(
            state.fail("provider_unavailable"),
            RagQueryState::Failed {
                query: "capital".into(),
                message: "provider_unavailable".into(),
            }
        );
    }
}
