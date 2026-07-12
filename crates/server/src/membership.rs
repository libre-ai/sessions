//! SP-A membership and personal-space authority.
//!
//! OIDC establishes an external subject only. This store remains the local
//! source of truth for space ownership and membership. The in-memory
//! implementation makes personal-space bootstrap atomic and idempotent within a
//! server process; a durable multi-instance implementation remains a later
//! storage adapter, not a different authorization model.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::authz::AuthzError;

/// Ordered membership roles (`viewer < contributor < inviter < admin < owner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Contributor,
    Inviter,
    Admin,
    Owner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    pub role: Role,
}

/// Server-owned descriptor for the one personal space associated with a subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSpace {
    pub id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct MembershipError(pub String);

impl std::fmt::Display for MembershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "membership store unavailable")
    }
}

impl std::error::Error for MembershipError {}

/// Authoritative local membership seam. A personal-space bootstrap and its
/// owner membership are one operation so concurrent first logins cannot create
/// two spaces or observe an ownerless space.
#[async_trait]
pub trait MembershipStore: Send + Sync {
    async fn member(&self, space: &str, sub: &str) -> Option<Membership>;
    async fn upsert_member(
        &self,
        space: &str,
        sub: &str,
        role: Role,
    ) -> Result<(), MembershipError>;
    async fn revoke_member(&self, space: &str, sub: &str) -> Result<(), MembershipError>;
    async fn list_members(&self, space: &str)
    -> Result<Vec<(String, Membership)>, MembershipError>;
    async fn ensure_personal_space(&self, sub: &str) -> Result<PersonalSpace, MembershipError>;
    async fn personal_space(&self, sub: &str) -> Result<Option<PersonalSpace>, MembershipError>;
}

#[derive(Default)]
struct InMemoryMembershipState {
    members: HashMap<(String, String), Membership>,
    personal_spaces: HashMap<String, PersonalSpace>,
}

/// Single-process adapter used by local development and deterministic tests.
#[derive(Default)]
pub struct InMemoryMembershipStore {
    state: Mutex<InMemoryMembershipState>,
}

impl InMemoryMembershipStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MembershipStore for InMemoryMembershipStore {
    async fn member(&self, space: &str, sub: &str) -> Option<Membership> {
        self.state
            .lock()
            .members
            .get(&(space.to_string(), sub.to_string()))
            .cloned()
    }

    async fn upsert_member(
        &self,
        space: &str,
        sub: &str,
        role: Role,
    ) -> Result<(), MembershipError> {
        self.state
            .lock()
            .members
            .insert((space.to_string(), sub.to_string()), Membership { role });
        Ok(())
    }

    async fn revoke_member(&self, space: &str, sub: &str) -> Result<(), MembershipError> {
        self.state
            .lock()
            .members
            .remove(&(space.to_string(), sub.to_string()));
        Ok(())
    }

    async fn list_members(
        &self,
        space: &str,
    ) -> Result<Vec<(String, Membership)>, MembershipError> {
        Ok(self
            .state
            .lock()
            .members
            .iter()
            .filter(|((candidate, _), _)| candidate == space)
            .map(|((_, sub), membership)| (sub.clone(), membership.clone()))
            .collect())
    }

    async fn ensure_personal_space(&self, sub: &str) -> Result<PersonalSpace, MembershipError> {
        if sub.is_empty() || sub.len() > 512 {
            return Err(MembershipError("invalid subject".into()));
        }
        let mut state = self.state.lock();
        if let Some(existing) = state.personal_spaces.get(sub).cloned() {
            // Heal the local invariant if a lower-level test adapter operation
            // removed or changed the owner membership.
            state.members.insert(
                (existing.id.clone(), sub.to_string()),
                Membership { role: Role::Owner },
            );
            return Ok(existing);
        }

        let space = PersonalSpace {
            id: format!("space_{}", uuid::Uuid::new_v4().simple()),
            name: "Espace personnel".to_string(),
        };
        state.personal_spaces.insert(sub.to_string(), space.clone());
        state.members.insert(
            (space.id.clone(), sub.to_string()),
            Membership { role: Role::Owner },
        );
        Ok(space)
    }

    async fn personal_space(&self, sub: &str) -> Result<Option<PersonalSpace>, MembershipError> {
        Ok(self.state.lock().personal_spaces.get(sub).cloned())
    }
}

/// Re-check membership for a sensitive operation. A revoked or absent member is
/// denied regardless of any still-valid capability.
pub async fn recheck_sensitive(
    store: &dyn MembershipStore,
    space: &str,
    sub: &str,
) -> Result<Role, AuthzError> {
    store
        .member(space, sub)
        .await
        .map(|membership| membership.role)
        .ok_or(AuthzError::Revoked)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn revoked_member_is_denied_a_sensitive_op_despite_a_valid_token() {
        let store = InMemoryMembershipStore::new();
        store
            .upsert_member("space-A", "user-1", Role::Admin)
            .await
            .unwrap();
        assert_eq!(
            recheck_sensitive(&store, "space-A", "user-1").await,
            Ok(Role::Admin)
        );
        store.revoke_member("space-A", "user-1").await.unwrap();
        assert_eq!(
            recheck_sensitive(&store, "space-A", "user-1").await,
            Err(AuthzError::Revoked)
        );
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_updates_role() {
        let store = InMemoryMembershipStore::new();
        store.upsert_member("s", "u", Role::Viewer).await.unwrap();
        store.upsert_member("s", "u", Role::Admin).await.unwrap();
        assert_eq!(store.member("s", "u").await.unwrap().role, Role::Admin);
        assert_eq!(store.list_members("s").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn concurrent_bootstrap_creates_exactly_one_owner_space() {
        let store = Arc::new(InMemoryMembershipStore::new());
        let mut tasks = Vec::new();
        for _ in 0..32 {
            let store = store.clone();
            tasks.push(tokio::spawn(async move {
                store.ensure_personal_space("subject-1").await.unwrap()
            }));
        }
        let spaces = futures_util::future::join_all(tasks).await;
        let first = spaces[0].as_ref().unwrap();
        assert!(spaces.iter().all(|space| space.as_ref().unwrap() == first));
        assert_eq!(store.list_members(&first.id).await.unwrap().len(), 1);
        assert_eq!(
            store.member(&first.id, "subject-1").await.unwrap().role,
            Role::Owner
        );
    }
}
