//! In-memory session registry with a per-session broadcast fanout.
//!
//! This is the concrete single-instance seam. A `SessionStore` / `Fanout` trait
//! is deliberately NOT introduced yet: abstracting before the second (Redis /
//! Postgres) implementation would be a premature, likely-wrong abstraction. The
//! trait is extracted in TB-2 when that second impl exists (rule of three).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;

use presto_core::protocol::ServerMessage;

use crate::session::Session;

/// Broadcast buffer per session. 2048 (not 1024) after the adversarial review
/// observed saturation at 500 concurrent — see `docs/plans/`.
const BROADCAST_CAPACITY: usize = 2048;

/// A live session plus the channel that fans server messages out to its sockets.
pub struct SessionHandle {
    pub session: Mutex<Session>,
    pub tx: broadcast::Sender<ServerMessage>,
}

/// Thread-safe registry of live sessions. Cheap to clone (shared `Arc`).
#[derive(Clone, Default)]
pub struct SessionRegistry {
    sessions: Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the session, creating it (with `host_id` as host) if absent.
    pub fn get_or_create(&self, session_id: &str, host_id: &str) -> Arc<SessionHandle> {
        let mut map = self.sessions.lock();
        map.entry(session_id.to_string())
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
                Arc::new(SessionHandle {
                    session: Mutex::new(Session::new(session_id, host_id)),
                    tx,
                })
            })
            .clone()
    }

    /// Number of live sessions (used in tests / metrics).
    pub fn len(&self) -> usize {
        self.sessions.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_create_is_idempotent() {
        let reg = SessionRegistry::new();
        let a = reg.get_or_create("s1", "host");
        let b = reg.get_or_create("s1", "host");
        assert!(Arc::ptr_eq(&a, &b), "same session handle reused");
        assert_eq!(reg.len(), 1);
        reg.get_or_create("s2", "host");
        assert_eq!(reg.len(), 2);
    }
}
