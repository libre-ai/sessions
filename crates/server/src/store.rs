//! The session-state seam: where authoritative session state lives.
//!
//! In-memory today ([`InMemorySessionStore`]); a Postgres-backed store (for
//! crash recovery across instances) plugs in behind the same trait in a later
//! slice. State is returned as `Arc<Mutex<Session>>` so the WS handler mutates
//! it under a short synchronous lock.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::session::Session;

/// Look up or create authoritative session state.
pub trait SessionStore: Send + Sync {
    fn get_or_create(&self, session_id: &str, host_id: &str) -> Arc<Mutex<Session>>;
}

/// Single-instance, in-memory session store.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live sessions (tests / metrics).
    pub fn len(&self) -> usize {
        self.sessions.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SessionStore for InMemorySessionStore {
    fn get_or_create(&self, session_id: &str, host_id: &str) -> Arc<Mutex<Session>> {
        self.sessions
            .lock()
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(Session::new(session_id, host_id))))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_create_is_idempotent() {
        let store = InMemorySessionStore::new();
        let a = store.get_or_create("s1", "host");
        let b = store.get_or_create("s1", "host");
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(store.len(), 1);
        store.get_or_create("s2", "host");
        assert_eq!(store.len(), 2);
    }
}
