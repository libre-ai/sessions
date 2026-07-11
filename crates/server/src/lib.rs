//! presto-server — the Presto-Matic backend library.
//!
//! The authoritative live-session engine ([`session`]), the seams for state and
//! fanout ([`store`], [`fanout`]), the Biscuit join-link authorization ([`auth`]),
//! and the WebSocket handler ([`ws`]) live here as testable library code;
//! `src/main.rs` is the thin binary entry point. The [`store::SessionStore`] and
//! [`fanout::Fanout`] traits are the seams where the distributed (Redis /
//! Postgres) implementations plug in for multi-instance operation.

pub mod auth;
pub mod authz;
pub mod classification;
pub mod fanout;
pub mod flashcard_store;
pub mod grounded_fixtures;
pub mod http;
pub mod ingestion;
pub mod integrity;
pub mod jobs;
pub mod membership;
pub mod oidc;
pub mod postgres_store;
pub mod quiz;
pub mod ratelimit;
pub mod redis_fanout;
pub mod scoring;
pub mod session;
pub mod session_identity;
pub mod store;
pub mod ws;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use auth::Auth;
use fanout::{BroadcastFanout, Fanout};
use quiz::{
    BreakoutSource, DocumentIngestor, FixtureBreakoutSource, FixtureFlashcardSource,
    FixtureIngestor, FixtureQuizSource, FlashcardSource, QuizSource,
};
use ratelimit::TokenBucket;
use store::{InMemorySessionStore, SessionStore};

/// Public exports for the scoring hook interface.
/// See [`scoring::ScoreSink`] for trait definition and examples.
pub use scoring::{InMemorySink, ScoreError, ScoreSink};

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
    pub ingestor: Arc<dyn DocumentIngestor>,
    /// Guards the open `POST /sessions` endpoint against creation spam.
    pub session_rate: Arc<TokenBucket>,
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
            ingestor: Arc::new(FixtureIngestor),
            // Generous burst, ~1 new session/sec sustained; tune via env in main.
            session_rate: Arc::new(TokenBucket::new(30.0, 1.0)),
        }
    }
}

/// Build the application router over shared state.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(http::index))
        .route("/app.js", get(http::app_js))
        .route("/health", get(health))
        .route("/p0/contract/proof", get(http::p0_contract_proof))
        .route("/p0/stub/run", post(http::p0_stub_run))
        .route("/sessions", post(http::create_session))
        .route(
            "/sessions/{session_id}/participants",
            post(http::join_session),
        )
        .route("/corpus/documents", post(http::ingest_document))
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

    async fn ingest(uri: &str, content_type: &str, body: &'static str) -> StatusCode {
        let state = AppState::in_memory(Arc::new(Auth::generate()));
        app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn p0_contract_proof_is_green_and_runtime_free() {
        let state = AppState::in_memory(Arc::new(Auth::generate()));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/p0/contract/proof")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["report"]["valid"], true);
        assert_eq!(json["data"]["execution"]["llmProviderCalled"], false);
        assert_eq!(json["data"]["execution"]["wrenchCalled"], false);
        assert_eq!(json["data"]["execution"]["gearCalled"], false);
        assert_eq!(json["data"]["execution"]["boltCalled"], false);
        assert_eq!(json["data"]["execution"]["biscuitRuntimeCalled"], false);
    }

    #[tokio::test]
    async fn p0_stub_run_returns_vertical_steps_without_runtime_calls() {
        let state = AppState::in_memory(Arc::new(Auth::generate()));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/p0/stub/run")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["valid"], true);
        assert_eq!(json["data"]["fixtureValid"], true);
        assert_eq!(json["data"]["execution"]["durableStorageWritten"], false);
        assert_eq!(json["data"]["execution"]["llmProviderCalled"], false);
        let steps = json["data"]["steps"].as_array().unwrap();
        assert!(steps.iter().any(|step| step["name"] == "attach_sources"));
        assert!(
            steps
                .iter()
                .any(|step| step["name"] == "export_participant_artifact")
        );
        assert!(steps.iter().all(|step| step["ok"] == true));
    }

    #[tokio::test]
    async fn ingest_without_corpus_is_unavailable() {
        // The default in-memory state uses FixtureIngestor → not configured (503,
        // not a client error, and no backend detail leaked).
        let status = ingest(
            "/corpus/documents?document_id=doc1",
            "text/markdown",
            "# Hi\n\nBody",
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn ingest_requires_a_document_id() {
        let status = ingest("/corpus/documents?document_id=", "text/plain", "x").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_session_is_rate_limited() {
        let mut state = AppState::in_memory(Arc::new(Auth::generate()));
        // An empty, non-refilling bucket refuses the very first creation.
        state.session_rate = Arc::new(ratelimit::TokenBucket::new(0.0, 0.0));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn ingest_rejects_an_overlong_document_id() {
        let long = "a".repeat(200);
        let status = ingest(
            &format!("/corpus/documents?document_id={long}"),
            "text/plain",
            "x",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
