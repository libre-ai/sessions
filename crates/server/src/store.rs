//! The session-state seam, as **async operations** so authoritative state can
//! live off-instance (Postgres) for correct cross-instance scoring.
//!
//! [`InMemorySessionStore`] wraps the in-memory [`Session`] engine (single
//! instance); [`crate::postgres_store::PostgresSessionStore`] shares state across
//! instances. Answer submission is accepted only for the exact open question,
//! and reveal is idempotent: the score mutates at most once per round. The WS
//! handler only ever calls these operations — never touches state directly — so
//! swapping the backend needs no handler change.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use presto_core::protocol::{Question, QuestionPublic, SessionSnapshot};

use crate::session::{RevealResult, SectionMastery, Session, SessionError};

/// A store operation failure.
#[derive(Debug)]
pub enum StoreError {
    /// A live-session rule was violated (wrong phase, double answer, …).
    Session(SessionError),
    /// The backend (database, network) failed.
    Backend(String),
}

impl StoreError {
    pub fn client_reason(&self) -> &'static str {
        match self {
            StoreError::Session(e) => e.client_reason(),
            StoreError::Backend(_) => "backend_error",
        }
    }
}

impl From<SessionError> for StoreError {
    fn from(e: SessionError) -> Self {
        StoreError::Session(e)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Session(e) => write!(f, "{e:?}"),
            StoreError::Backend(m) => write!(f, "store backend: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub type StoreResult<T> = Result<T, StoreError>;

/// Authoritative session state as a set of async operations.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Create the session if absent (idempotent), with `host_id` as host.
    async fn ensure(&self, session_id: &str, host_id: &str) -> StoreResult<()>;
    /// Add (or re-add) a participant; returns the participant count.
    async fn join(&self, session_id: &str, participant_id: &str, name: &str) -> StoreResult<u32>;
    /// Open a question at `opened_at_ms` (host action): clears prior answers,
    /// enters `Asking`.
    async fn push_question(
        &self,
        session_id: &str,
        question: &Question,
        opened_at_ms: u64,
    ) -> StoreResult<()>;
    /// Record a participant's answer (once, while `Asking`, before the deadline).
    /// `question_id` must match the open question; `now_ms` is the server
    /// clock and elapsed time is computed server-side.
    async fn submit_answer(
        &self,
        session_id: &str,
        participant_id: &str,
        question_id: &str,
        choices: Vec<u8>,
        now_ms: u64,
    ) -> StoreResult<()>;
    /// The currently open question (public projection), for a participant joining
    /// or reconnecting mid-question.
    async fn snapshot(&self, session_id: &str) -> StoreResult<Option<QuestionPublic>>;
    /// A personalized reconnect snapshot, including the current participant's
    /// answered state and the cached reveal when the round is already closed.
    async fn guest_snapshot(
        &self,
        session_id: &str,
        participant_id: &str,
    ) -> StoreResult<Option<SessionSnapshot>>;
    /// Whether the session exists (so a participant token is only minted for a
    /// real session).
    async fn exists(&self, session_id: &str) -> StoreResult<bool>;
    /// A participant's per-section mastery accumulated across the session.
    async fn mastery(
        &self,
        session_id: &str,
        participant_id: &str,
    ) -> StoreResult<Vec<SectionMastery>>;
    /// Score the round and return the leaderboard + heatmap; enters `Revealed`.
    async fn reveal(&self, session_id: &str) -> StoreResult<RevealResult>;
}

/// Single-instance, in-memory store: wraps the pure [`Session`] engine.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_or_create(&self, session_id: &str, host_id: &str) -> Arc<Mutex<Session>> {
        self.sessions
            .lock()
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(Session::new(session_id, host_id))))
            .clone()
    }

    /// Number of live sessions (tests / metrics).
    pub fn len(&self) -> usize {
        self.sessions.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn ensure(&self, session_id: &str, host_id: &str) -> StoreResult<()> {
        self.get_or_create(session_id, host_id);
        Ok(())
    }

    async fn join(&self, session_id: &str, participant_id: &str, name: &str) -> StoreResult<u32> {
        Ok(self
            .get_or_create(session_id, "")
            .lock()
            .join(participant_id, name))
    }

    async fn push_question(
        &self,
        session_id: &str,
        question: &Question,
        opened_at_ms: u64,
    ) -> StoreResult<()> {
        self.get_or_create(session_id, "")
            .lock()
            .push_question(question.clone(), opened_at_ms);
        Ok(())
    }

    async fn submit_answer(
        &self,
        session_id: &str,
        participant_id: &str,
        question_id: &str,
        choices: Vec<u8>,
        now_ms: u64,
    ) -> StoreResult<()> {
        self.get_or_create(session_id, "").lock().submit_answer(
            question_id,
            participant_id,
            choices,
            now_ms,
        )?;
        Ok(())
    }

    async fn snapshot(&self, session_id: &str) -> StoreResult<Option<QuestionPublic>> {
        Ok(self.get_or_create(session_id, "").lock().open_question())
    }

    async fn guest_snapshot(
        &self,
        session_id: &str,
        participant_id: &str,
    ) -> StoreResult<Option<SessionSnapshot>> {
        let session = self.sessions.lock().get(session_id).cloned();
        match session {
            Some(session) => session
                .lock()
                .guest_snapshot(participant_id)
                .map(Some)
                .map_err(StoreError::Backend),
            None => Ok(None),
        }
    }

    async fn exists(&self, session_id: &str) -> StoreResult<bool> {
        Ok(self.sessions.lock().contains_key(session_id))
    }

    async fn mastery(
        &self,
        session_id: &str,
        participant_id: &str,
    ) -> StoreResult<Vec<SectionMastery>> {
        Ok(self
            .get_or_create(session_id, "")
            .lock()
            .mastery(participant_id))
    }

    async fn reveal(&self, session_id: &str) -> StoreResult<RevealResult> {
        let result = self.get_or_create(session_id, "").lock().reveal()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presto_core::protocol::{Question, QuestionKind};

    fn question() -> Question {
        Question {
            id: "q1".into(),
            text: "?".into(),
            kind: QuestionKind::Single,
            choices: vec!["a".into(), "b".into()],
            correct_choices: vec![1],
            source_section_ids: vec!["s1".into()],
            citation_validation: None,
            timer_sec: 20,
        }
    }

    #[tokio::test]
    async fn full_round_through_the_store() {
        let store = InMemorySessionStore::new();
        store.ensure("s1", "host").await.unwrap();
        assert_eq!(store.join("s1", "p1", "Alice").await.unwrap(), 1);
        store.push_question("s1", &question(), 0).await.unwrap();
        // The open question is available as a snapshot for late joiners.
        assert!(store.snapshot("s1").await.unwrap().is_some());
        let asking = store.guest_snapshot("s1", "p1").await.unwrap().unwrap();
        assert_eq!(
            asking.phase,
            presto_core::protocol::SessionPhasePublic::Asking
        );
        assert!(asking.question.is_some());
        assert!(asking.reveal.is_none());
        assert!(!asking.answered);
        store
            .submit_answer("s1", "p1", "q1", vec![1], 1000)
            .await
            .unwrap();
        // double answer is rejected by the engine, surfaced as a store error.
        assert!(
            store
                .submit_answer("s1", "p1", "q1", vec![0], 1)
                .await
                .is_err()
        );
        let reveal = store.reveal("s1").await.unwrap();
        assert_eq!(reveal.correct_choices, vec![1]);
        assert_eq!(reveal.leaderboard[0].participant_id, "p1");
        assert!(reveal.leaderboard[0].score >= 500);
        // Repeated reveal is immutable.
        assert_eq!(store.reveal("s1").await.unwrap(), reveal);
        // After reveal, legacy snapshot is closed but the personalized snapshot
        // still carries the cached reveal for reconnects.
        assert!(store.snapshot("s1").await.unwrap().is_none());
        let revealed = store.guest_snapshot("s1", "p1").await.unwrap().unwrap();
        assert_eq!(
            revealed.phase,
            presto_core::protocol::SessionPhasePublic::Revealed
        );
        assert!(revealed.question.is_some());
        assert!(revealed.reveal.is_some());
        assert!(revealed.answered);
    }
}
