//! presto-server — the Presto-Matic backend library.
//!
//! The authoritative live-session engine ([`session`]), the seams for state and
//! fanout ([`store`], [`fanout`]), the Biscuit join-link authorization ([`auth`]),
//! and the WebSocket handler ([`ws`]) live here as testable library code;
//! `src/main.rs` is the thin binary entry point. The [`store::SessionStore`] and
//! [`fanout::Fanout`] traits are the seams where the distributed (Redis /
//! Postgres) implementations plug in for multi-instance operation.

pub mod approved_claims;
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
pub mod notebook_rag;
pub mod oidc;
pub mod owner_auth;
pub mod owner_corpus;
pub mod owner_corpus_http;
pub mod postgres_jobs;
pub mod postgres_store;
pub mod quiz;
pub mod rag_query;
pub mod ratelimit;
pub mod redis_fanout;
pub mod scoring;
pub mod session;
pub mod session_identity;
pub mod store;
pub mod ws;

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tower::limit::ConcurrencyLimitLayer;

use approved_claims::ApprovedClaimRegistry;
use auth::Auth;
use fanout::{BroadcastFanout, Fanout};
use notebook_rag::{NotebookRagEngine, StagedNotebookRagEngine};
use owner_auth::OwnerAuth;
use owner_corpus::OwnerCorpusStore;
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
    /// Bounded, process-local owner corpus shared by list/upload and retrieval.
    pub owner_corpus: Arc<OwnerCorpusStore>,
    /// Immutable, server-side authority for publishable notebook claims.
    pub approved_claims: Arc<ApprovedClaimRegistry>,
    /// Untrusted retrieve/generate/verify stages; never an approval authority.
    pub notebook_rag: Arc<dyn NotebookRagEngine>,
    pub quiz: Arc<dyn QuizSource>,
    pub breakout: Arc<dyn BreakoutSource>,
    pub flashcards: Arc<dyn FlashcardSource>,
    pub ingestor: Arc<dyn DocumentIngestor>,
    /// Strong bearer for the isolated legacy live-RAG ingestion boundary.
    /// `None` is fail-closed and never means public access.
    pub legacy_ingest_token: Option<Arc<str>>,
    /// Guards the open `POST /sessions` endpoint against creation spam.
    pub session_rate: Arc<TokenBucket>,
}

impl AppState {
    /// Single-instance state: in-memory store + tokio-broadcast fanout +
    /// fixture-backed content (no AI provider or corpus required).
    pub fn in_memory(auth: Arc<Auth>) -> Self {
        let owner_corpus = Arc::new(OwnerCorpusStore::new());
        Self {
            store: Arc::new(InMemorySessionStore::new()),
            fanout: Arc::new(BroadcastFanout::new()),
            owner_auth: Arc::new(OwnerAuth::disabled(auth.clone())),
            approved_claims: Arc::new(ApprovedClaimRegistry::with_owner_corpus(
                owner_corpus.clone(),
            )),
            notebook_rag: Arc::new(StagedNotebookRagEngine::fixture_with_owner_corpus(
                owner_corpus.clone(),
            )),
            owner_corpus,
            auth,
            quiz: Arc::new(FixtureQuizSource),
            breakout: Arc::new(FixtureBreakoutSource),
            flashcards: Arc::new(FixtureFlashcardSource),
            ingestor: Arc::new(FixtureIngestor),
            legacy_ingest_token: None,
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
        .route(
            "/api/rag/query",
            post(rag_query::query)
                // Axum layers execute bottom-up: auth is outermost, then the
                // shared permit around body buffering and the complete pipeline.
                .layer::<_, Infallible>(DefaultBodyLimit::max(rag_query::MAX_RAG_BODY_BYTES))
                .layer::<_, Infallible>(ConcurrencyLimitLayer::new(
                    rag_query::MAX_CONCURRENT_QUERIES,
                ))
                .layer::<_, Infallible>(middleware::from_fn_with_state(
                    state.clone(),
                    rag_query::authorize_query,
                )),
        )
        .route(
            "/api/corpus/documents",
            get(owner_corpus_http::list).merge(
                post(owner_corpus_http::upload)
                    // Axum layers execute bottom-up: auth is outermost, then the
                    // shared concurrency permit, then body buffering/extraction.
                    .layer::<_, Infallible>(DefaultBodyLimit::max(
                        owner_corpus_http::MAX_DOCUMENT_BODY_BYTES,
                    ))
                    .layer::<_, Infallible>(ConcurrencyLimitLayer::new(
                        owner_corpus_http::MAX_CONCURRENT_UPLOADS,
                    ))
                    .layer::<_, Infallible>(middleware::from_fn_with_state(
                        state.clone(),
                        owner_corpus_http::authorize_upload,
                    )),
            ),
        )
        .route("/p0/contract/proof", get(http::p0_contract_proof))
        .route("/p0/stub/run", post(http::p0_stub_run))
        .route("/sessions", post(http::create_session))
        .route(
            "/sessions/{session_id}/participants",
            post(http::join_session),
        )
        .route(
            "/corpus/documents",
            post(http::ingest_document)
                .layer::<_, Infallible>(DefaultBodyLimit::max(http::MAX_LEGACY_INGEST_BODY_BYTES))
                .layer::<_, Infallible>(ConcurrencyLimitLayer::new(
                    http::MAX_CONCURRENT_LEGACY_INGESTS,
                ))
                .layer::<_, Infallible>(middleware::from_fn_with_state(
                    state.clone(),
                    http::authorize_legacy_ingest,
                )),
        )
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
    use std::convert::Infallible;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::{Body, Bytes};
    use axum::http::{Request, StatusCode};
    use futures_util::stream;
    use tower::ServiceExt;

    #[test]
    fn grounded_projection_is_confined_to_the_authority_module() {
        fn visit(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, files);
                } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }

        let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let authority = source_root.join("approved_claims.rs");
        let direct_variant = ["RagQueryResponse", "::Grounded"].concat();
        let convenience_constructor = ["RagQueryResponse", "::grounded("].concat();
        let mut files = Vec::new();
        visit(&source_root, &mut files);
        let violations: Vec<_> = files
            .into_iter()
            .filter(|path| path != &authority)
            .filter(|path| {
                let source = std::fs::read_to_string(path).unwrap();
                source.contains(&direct_variant) || source.contains(&convenience_constructor)
            })
            .collect();
        assert!(
            violations.is_empty(),
            "Grounded may only be constructed by approved_claims.rs: {violations:?}"
        );
    }

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

    const TEST_INGEST_TOKEN: &str = "0123456789abcdef0123456789abcdef";

    async fn ingest(uri: &str, content_type: &str, body: &'static str) -> StatusCode {
        let mut state = AppState::in_memory(Arc::new(Auth::generate()));
        state.legacy_ingest_token = Some(Arc::from(TEST_INGEST_TOKEN));
        app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", content_type)
                    .header("authorization", format!("Bearer {TEST_INGEST_TOKEN}"))
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
    async fn legacy_ingest_fails_closed_before_polling_body() {
        let state = AppState::in_memory(Arc::new(Auth::generate()));
        let polls = Arc::new(AtomicUsize::new(0));
        let observed = polls.clone();
        let body = Body::from_stream(stream::once(async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(Bytes::from_static(b"secret body"))
        }));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/corpus/documents?document_id=doc1")
                    .header("content-type", "text/plain")
                    .header("authorization", "Bearer wrong-token")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(polls.load(Ordering::SeqCst), 0);

        let mut configured = AppState::in_memory(Arc::new(Auth::generate()));
        configured.legacy_ingest_token = Some(Arc::from(TEST_INGEST_TOKEN));
        let wrong = app(configured)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/corpus/documents?document_id=doc1")
                    .header("authorization", "Bearer definitely-wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(wrong.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn legacy_ingest_body_is_bounded_after_authentication() {
        let mut state = AppState::in_memory(Arc::new(Auth::generate()));
        state.legacy_ingest_token = Some(Arc::from(TEST_INGEST_TOKEN));
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/corpus/documents?document_id=doc1")
                    .header("content-type", "text/plain")
                    .header("authorization", format!("Bearer {TEST_INGEST_TOKEN}"))
                    .body(Body::from(vec![
                        b'x';
                        http::MAX_LEGACY_INGEST_BODY_BYTES + 1
                    ]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
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
