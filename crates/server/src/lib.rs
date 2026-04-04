//! presto-server — the Presto-Matic backend library.
//!
//! The authoritative live-session engine ([`session`]), the seams for state and
//! fanout ([`store`], [`fanout`]), the Biscuit join-link authorization ([`auth`]),
//! and the WebSocket handler ([`ws`]) live here as testable library code;
//! `src/main.rs` is the thin binary entry point. The [`store::SessionStore`] and
//! [`fanout::Fanout`] traits are the seams where the distributed (Redis /
//! Postgres) implementations plug in for multi-instance operation.

pub mod auth;
pub mod fanout;
pub mod http;
pub mod postgres_store;
pub mod quiz;
pub mod redis_fanout;
pub mod session;
pub mod store;
pub mod ws;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use auth::Auth;
use fanout::{BroadcastFanout, Fanout};
use quiz::{
    BreakoutSource, FixtureBreakoutSource, FixtureFlashcardSource, FixtureQuizSource,
    FlashcardSource, QuizSource,
};
use store::{InMemorySessionStore, SessionStore};

/// Shared application state: the session-state store, the fanout, the token
/// authority, and the quiz/breakout content sources. The trait objects let a
/// deployment choose single- vs multi-instance (Redis/Postgres) and fixture vs
/// RAG-backed content at startup.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn SessionStore>,
    pub fanout: Arc<dyn Fanout>,
    pub auth: Arc<Auth>,
    pub quiz: Arc<dyn QuizSource>,
    pub breakout: Arc<dyn BreakoutSource>,
    pub flashcards: Arc<dyn FlashcardSource>,
}

impl AppState {
    /// Single-instance state: in-memory store + tokio-broadcast fanout +
    /// fixture-backed content (no AI provider or corpus required).
    pub fn in_memory(auth: Arc<Auth>) -> Self {
        Self {
            store: Arc::new(InMemorySessionStore::new()),
            fanout: Arc::new(BroadcastFanout::new()),
            auth,
            quiz: Arc::new(FixtureQuizSource),
            breakout: Arc::new(FixtureBreakoutSource),
            flashcards: Arc::new(FixtureFlashcardSource),
        }
    }
}

/// Build the application router over shared state.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(http::index))
        .route("/app.js", get(http::app_js))
        .route("/health", get(health))
        .route("/sessions", post(http::create_session))
        .route(
            "/sessions/{session_id}/participants",
            post(http::join_session),
        )
        .route("/ws/{session_id}", get(ws::ws_handler))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_returns_ok() {
        let state = AppState::in_memory(Arc::new(Auth::generate()));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
