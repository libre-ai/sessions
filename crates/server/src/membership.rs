//! SP-A `MembershipStore`: the single source of truth for who is a member of
//! which space, with which role — and therefore the basis of **immediate
//! revocation**. A sensitive op (`add_document`, `invite`, `manage_members`, …)
//! rechecks membership here, so a revoked member is denied regardless of an
//! unexpired token's TTL (the token is a capability, never a permission cache).
//!
//! In production the recheck rides a short fanout-invalidated cache (SP-A §E) to
//! keep the hot path off the database; that optimization is Increment 2. This is
//! the seam + the recheck rule, proven against the in-memory store.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::authz::AuthzError;

/// A space member's role. Ordered: a higher role includes the lower roles'
/// capabilities (`viewer < contributor < inviter < admin < owner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Contributor,
    Inviter,
    Admin,
    Owner,
}

/// A current membership record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    pub role: Role,
}

/// A membership-store backend failure.
#[derive(Debug)]
pub struct MembershipError(pub String);

impl std::fmt::Display for MembershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "membership store: {}", self.0)
    }
}

impl std::error::Error for MembershipError {}

/// Authoritative space membership. `member` returning `None` means absent **or**
/// revoked — both deny — which is what makes revocation immediate.
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
}

/// Single-instance, in-memory membership (tests / local). The Postgres-backed
/// store is the multi-instance production impl (Increment 2).
#[derive(Default)]
pub struct InMemoryMembershipStore {
    members: Mutex<HashMap<(String, String), Membership>>,
}

impl InMemoryMembershipStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MembershipStore for InMemoryMembershipStore {
    async fn member(&self, space: &str, sub: &str) -> Option<Membership> {
        self.members
            .lock()
            .get(&(space.to_string(), sub.to_string()))
            .cloned()
    }

    async fn upsert_member(
        &self,
        space: &str,
        sub: &str,
        role: Role,
    ) -> Result<(), MembershipError> {
        self.members
            .lock()
            .insert((space.to_string(), sub.to_string()), Membership { role });
        Ok(())
    }

    async fn revoke_member(&self, space: &str, sub: &str) -> Result<(), MembershipError> {
        self.members
            .lock()
            .remove(&(space.to_string(), sub.to_string()));
        Ok(())
    }

    async fn list_members(
        &self,
        space: &str,
    ) -> Result<Vec<(String, Membership)>, MembershipError> {
        Ok(self
            .members
            .lock()
            .iter()
            .filter(|((s, _), _)| s == space)
            .map(|((_, sub), m)| (sub.clone(), m.clone()))
            .collect())
    }
}

/// Re-check membership for a sensitive op: a revoked/absent member is denied
/// regardless of an unexpired token (immediate revocation). Returns the current
/// role on success.
pub async fn recheck_sensitive(
    store: &dyn MembershipStore,
    space: &str,
    sub: &str,
) -> Result<Role, AuthzError> {
    store
        .member(space, sub)
        .await
        .map(|m| m.role)
        .ok_or(AuthzError::Revoked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn revoked_member_is_denied_a_sensitive_op_despite_a_valid_token() {
        let store = InMemoryMembershipStore::new();
        store
            .upsert_member("space-A", "user-1", Role::Admin)
            .await
            .unwrap();

        // While a member, the sensitive-op recheck passes with the role.
        assert_eq!(
            recheck_sensitive(&store, "space-A", "user-1").await,
            Ok(Role::Admin)
        );

        // Revoke membership: the recheck now denies — the (still-unexpired) token
        // is a capability, not a cache, so revocation takes effect immediately.
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
}
