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
pub mod owner_auth;
pub mod postgres_jobs;
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
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use auth::Auth;
use fanout::{BroadcastFanout, Fanout};
use owner_auth::OwnerAuth;
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
    /// OIDC login transactions, opaque owner sessions and personal-space authz.
    pub owner_auth: Arc<OwnerAuth>,
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
            owner_auth: Arc::new(OwnerAuth::disabled(auth.clone())),
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
        .route("/app", get(http::owner_app_index))
        .route("/app/", get(http::owner_app_index))
        .route("/app/assets/{asset}", get(http::owner_app_asset))
        .route("/app/{*path}", get(http::owner_app_index))
        .route("/health", get(health))
        .route("/auth/login", get(owner_auth::login))
        .route("/auth/callback", get(owner_auth::callback))
        .route("/auth/logout", post(owner_auth::logout))
        .route("/api/me", get(owner_auth::me))
        .route("/api/spaces/current", get(owner_auth::current_space))
        .route("/p0/contract/proof", get(http::p0_contract_proof))
        .route("/p0/stub/run", post(http::p0_stub_run))
        .route("/sessions", post(http::create_session))
        .route(
            "/sessions/{session_id}/participants",
            post(http::join_session),
        )
        .route("/corpus/documents", post(http::ingest_document))
        .route("/ws/{session_id}", get(ws::ws_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_cookie_same_origin,
        ))
        .with_state(state)
}

/// Any unsafe request carrying the owner cookie must provide two independent
/// same-origin signals. This applies globally, including future unsafe `/api`
/// routes, rather than relying on each handler to remember the CSRF check.
async fn enforce_cookie_same_origin(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !safe
        && state.owner_auth.has_auth_cookie(request.headers())
        && !state
            .owner_auth
            .same_origin_cookie_request(request.headers())
    {
        return (
            StatusCode::FORBIDDEN,
            [(header::CACHE_CONTROL, "no-store")],
            "forbidden",
        )
            .into_response();
    }
    next.run(request).await
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

    #[tokio::test]
    async fn owner_app_and_nested_routes_serve_the_dioxus_shell() {
        for uri in ["/app", "/app/", "/app/notebook", "/app/corpus/deep-link"] {
            let state = AppState::in_memory(Arc::new(Auth::generate()));
            let response = app(state)
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                "text/html; charset=utf-8"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let html = String::from_utf8(body.to_vec()).unwrap();
            assert!(html.contains("<title>Rumble LM — espace owner</title>"));
            assert!(html.contains("/app/assets/rumble-lm-app-"));
            assert!(html.contains("<div id=\"main\"></div>"));
        }
    }

    #[tokio::test]
    async fn owner_app_serves_javascript_and_wasm_with_safe_content_types() {
        assert!(
            http::OWNER_APP_ASSETS
                .iter()
                .any(|asset| asset.content_type == "application/wasm")
        );
        for asset in http::OWNER_APP_ASSETS {
            let uri = format!("/app/assets/{}", asset.path);
            let state = AppState::in_memory(Arc::new(Auth::generate()));
            let response = app(state)
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                asset.content_type
            );
            assert_eq!(
                response.headers().get("x-content-type-options").unwrap(),
                "nosniff"
            );
        }
    }

    #[tokio::test]
    async fn unknown_owner_asset_is_not_replaced_by_the_html_fallback() {
        let state = AppState::in_memory(Arc::new(Auth::generate()));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/app/assets/missing.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
