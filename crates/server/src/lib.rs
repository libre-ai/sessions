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
    /// Guards the private join-link redemption endpoint against abuse.
    pub join_redemption_rate: Arc<TokenBucket>,
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
            // Join redemptions are separate from session creation and can be tuned independently.
            join_redemption_rate: Arc::new(TokenBucket::new(60.0, 2.0)),
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
        .route("/app/icons/{icon}", get(http::owner_app_icon))
        .route("/app/manifest.webmanifest", get(http::owner_app_manifest))
        .route("/app/sw.js", get(http::owner_app_service_worker))
        .route(
            "/app/owner-shell-manifest.json",
            get(http::owner_app_internal_manifest),
        )
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
            "/join/{session_id}/participants",
            post(http::redeem_join_link)
                .layer::<_, Infallible>(DefaultBodyLimit::max(http::MAX_JOIN_REDEMPTION_BODY_BYTES))
                // Authorization remains outside body extraction, while the
                // outer shared permit also bounds invalid-token verification.
                .layer::<_, Infallible>(middleware::from_fn_with_state(
                    state.clone(),
                    http::authorize_join_redemption,
                ))
                .layer::<_, Infallible>(ConcurrencyLimitLayer::new(
                    http::MAX_CONCURRENT_JOIN_REDEMPTIONS,
                )),
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
        .layer(middleware::from_fn(force_private_no_store))
        .with_state(state)
}

/// Dynamic identity/data boundaries are network-only. This centralized guard
/// applies to successes and errors and also prevents any cookie-setting response
/// from becoming cacheable when future routes are added.
async fn force_private_no_store(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path();
    let private_path = ["/auth", "/api", "/corpus", "/join", "/sessions", "/ws"]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")));
    let mut response = next.run(request).await;
    if private_path || response.headers().contains_key(header::SET_COOKIE) {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            "no-store".parse().expect("static cache control"),
        );
    }
    response
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::body::{Body, Bytes};
    use axum::http::{Request, StatusCode};
    use futures_util::stream;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use crate::store::{InMemorySessionStore, SessionStore};

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
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                http::OWNER_APP_CSP
            );
            assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
            assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
            assert!(response.headers().contains_key("permissions-policy"));
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let html = String::from_utf8(body.to_vec()).unwrap();
            assert!(html.contains("<title>Rumble LM — espace owner</title>"));
            assert!(html.contains("/app/assets/owner-runtime-"));
            assert!(!html.contains("/app/assets/rumble-lm-app-"));
            assert!(html.contains("<div id=\"main\"></div>"));
        }
    }

    #[tokio::test]
    async fn owner_app_static_files_have_exact_mime_cache_and_security_headers() {
        assert!(
            http::OWNER_APP_FILES
                .iter()
                .any(|asset| asset.content_type == "application/wasm")
        );
        for file in http::OWNER_APP_FILES {
            let uri = format!("/app/{}", file.path);
            let state = AppState::in_memory(Arc::new(Auth::generate()));
            let response = app(state)
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], file.content_type);
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            assert_eq!(
                response.headers()["cross-origin-resource-policy"],
                "same-origin"
            );
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                http::OWNER_APP_CSP
            );
            assert!(response.headers().contains_key(header::ETAG));
            if file.path.starts_with("assets/") {
                assert_eq!(
                    response.headers()[header::CACHE_CONTROL],
                    "public, max-age=31536000, immutable"
                );
            } else {
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
            }
            if file.path == "sw.js" {
                assert_eq!(response.headers()["service-worker-allowed"], "/app/");
            }
        }
    }

    #[tokio::test]
    async fn owner_service_worker_etag_supports_not_modified() {
        let file = http::OWNER_APP_FILES
            .iter()
            .find(|file| file.path == "sw.js")
            .unwrap();
        let response = app(AppState::in_memory(Arc::new(Auth::generate())))
            .oneshot(
                Request::builder()
                    .uri("/app/sw.js")
                    .header(header::IF_NONE_MATCH, format!("\"{}\"", file.etag))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[tokio::test]
    async fn any_set_cookie_response_is_forced_to_no_store() {
        async fn cookie_response() -> impl IntoResponse {
            (
                [
                    (header::SET_COOKIE, "future=value; HttpOnly"),
                    (header::CACHE_CONTROL, "public, max-age=60"),
                ],
                "ok",
            )
        }
        let response = Router::new()
            .route("/future", get(cookie_response))
            .layer(middleware::from_fn(force_private_no_store))
            .oneshot(
                Request::builder()
                    .uri("/future")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn dynamic_boundaries_are_no_store_on_success_and_error() {
        for (method, uri) in [
            ("GET", "/auth/login"),
            ("GET", "/api/me"),
            ("POST", "/corpus/documents"),
            ("POST", "/join/ABCDEF/participants"),
            ("POST", "/sessions"),
            ("GET", "/ws/missing"),
        ] {
            let response = app(AppState::in_memory(Arc::new(Auth::generate())))
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "no-store",
                "{uri}"
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

    struct BlockingJoinStore {
        inner: InMemorySessionStore,
        started: Arc<AtomicUsize>,
        released: Arc<AtomicBool>,
        release: Arc<Notify>,
    }

    impl BlockingJoinStore {
        fn new(started: Arc<AtomicUsize>, released: Arc<AtomicBool>, release: Arc<Notify>) -> Self {
            Self {
                inner: InMemorySessionStore::new(),
                started,
                released,
                release,
            }
        }
    }

    #[async_trait]
    impl crate::store::SessionStore for BlockingJoinStore {
        async fn ensure(&self, session_id: &str, host_id: &str) -> crate::store::StoreResult<()> {
            self.inner.ensure(session_id, host_id).await
        }

        async fn join(
            &self,
            session_id: &str,
            participant_id: &str,
            name: &str,
        ) -> crate::store::StoreResult<u32> {
            self.started.fetch_add(1, Ordering::SeqCst);
            while !self.released.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            self.inner.join(session_id, participant_id, name).await
        }

        async fn push_question(
            &self,
            session_id: &str,
            question: &presto_core::protocol::Question,
            opened_at_ms: u64,
        ) -> crate::store::StoreResult<()> {
            self.inner
                .push_question(session_id, question, opened_at_ms)
                .await
        }

        async fn submit_answer(
            &self,
            session_id: &str,
            participant_id: &str,
            question_id: &str,
            choices: Vec<u8>,
            now_ms: u64,
        ) -> crate::store::StoreResult<()> {
            self.inner
                .submit_answer(session_id, participant_id, question_id, choices, now_ms)
                .await
        }

        async fn snapshot(
            &self,
            session_id: &str,
        ) -> crate::store::StoreResult<Option<presto_core::protocol::QuestionPublic>> {
            self.inner.snapshot(session_id).await
        }

        async fn guest_snapshot(
            &self,
            session_id: &str,
            participant_id: &str,
        ) -> crate::store::StoreResult<Option<presto_core::protocol::SessionSnapshot>> {
            self.inner.guest_snapshot(session_id, participant_id).await
        }

        async fn exists(&self, session_id: &str) -> crate::store::StoreResult<bool> {
            self.inner.exists(session_id).await
        }

        async fn mastery(
            &self,
            session_id: &str,
            participant_id: &str,
        ) -> crate::store::StoreResult<Vec<crate::session::SectionMastery>> {
            self.inner.mastery(session_id, participant_id).await
        }

        async fn reveal(
            &self,
            session_id: &str,
        ) -> crate::store::StoreResult<crate::session::RevealResult> {
            self.inner.reveal(session_id).await
        }
    }

    struct FlakyJoinStore {
        inner: InMemorySessionStore,
        exists_calls: AtomicUsize,
    }

    impl FlakyJoinStore {
        fn new() -> Self {
            Self {
                inner: InMemorySessionStore::new(),
                exists_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl crate::store::SessionStore for FlakyJoinStore {
        async fn ensure(&self, session_id: &str, host_id: &str) -> crate::store::StoreResult<()> {
            self.inner.ensure(session_id, host_id).await
        }

        async fn join(
            &self,
            session_id: &str,
            participant_id: &str,
            name: &str,
        ) -> crate::store::StoreResult<u32> {
            self.inner.join(session_id, participant_id, name).await
        }

        async fn push_question(
            &self,
            session_id: &str,
            question: &presto_core::protocol::Question,
            opened_at_ms: u64,
        ) -> crate::store::StoreResult<()> {
            self.inner
                .push_question(session_id, question, opened_at_ms)
                .await
        }

        async fn submit_answer(
            &self,
            session_id: &str,
            participant_id: &str,
            question_id: &str,
            choices: Vec<u8>,
            now_ms: u64,
        ) -> crate::store::StoreResult<()> {
            self.inner
                .submit_answer(session_id, participant_id, question_id, choices, now_ms)
                .await
        }

        async fn snapshot(
            &self,
            session_id: &str,
        ) -> crate::store::StoreResult<Option<presto_core::protocol::QuestionPublic>> {
            self.inner.snapshot(session_id).await
        }

        async fn guest_snapshot(
            &self,
            session_id: &str,
            participant_id: &str,
        ) -> crate::store::StoreResult<Option<presto_core::protocol::SessionSnapshot>> {
            self.inner.guest_snapshot(session_id, participant_id).await
        }

        async fn exists(&self, _session_id: &str) -> crate::store::StoreResult<bool> {
            Ok(self.exists_calls.fetch_add(1, Ordering::SeqCst) == 0)
        }

        async fn mastery(
            &self,
            session_id: &str,
            participant_id: &str,
        ) -> crate::store::StoreResult<Vec<crate::session::SectionMastery>> {
            self.inner.mastery(session_id, participant_id).await
        }

        async fn reveal(
            &self,
            session_id: &str,
        ) -> crate::store::StoreResult<crate::session::RevealResult> {
            self.inner.reveal(session_id).await
        }
    }

    struct ErrorJoinStore;

    #[async_trait]
    impl crate::store::SessionStore for ErrorJoinStore {
        async fn ensure(&self, _session_id: &str, _host_id: &str) -> crate::store::StoreResult<()> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn join(
            &self,
            _session_id: &str,
            _participant_id: &str,
            _name: &str,
        ) -> crate::store::StoreResult<u32> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn push_question(
            &self,
            _session_id: &str,
            _question: &presto_core::protocol::Question,
            _opened_at_ms: u64,
        ) -> crate::store::StoreResult<()> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn submit_answer(
            &self,
            _session_id: &str,
            _participant_id: &str,
            _question_id: &str,
            _choices: Vec<u8>,
            _now_ms: u64,
        ) -> crate::store::StoreResult<()> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn snapshot(
            &self,
            _session_id: &str,
        ) -> crate::store::StoreResult<Option<presto_core::protocol::QuestionPublic>> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn guest_snapshot(
            &self,
            _session_id: &str,
            _participant_id: &str,
        ) -> crate::store::StoreResult<Option<presto_core::protocol::SessionSnapshot>> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn exists(&self, _session_id: &str) -> crate::store::StoreResult<bool> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn mastery(
            &self,
            _session_id: &str,
            _participant_id: &str,
        ) -> crate::store::StoreResult<Vec<crate::session::SectionMastery>> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }

        async fn reveal(
            &self,
            _session_id: &str,
        ) -> crate::store::StoreResult<crate::session::RevealResult> {
            Err(crate::store::StoreError::Backend(
                "backend unavailable".into(),
            ))
        }
    }

    const TEST_INGEST_TOKEN: &str = "0123456789abcdef0123456789abcdef";

    async fn join_redemption_response(
        state: AppState,
        request: Request<Body>,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let response = app(state).oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, String::from_utf8(body.to_vec()).unwrap())
    }

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
    async fn join_redemption_fails_closed_before_polling_body() {
        let auth = Arc::new(Auth::generate());
        let state = AppState::in_memory(auth);
        state.store.ensure("ABCDEF", "host").await.unwrap();
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
                    .uri("/join/ABCDEF/participants")
                    .header("authorization", "Bearer definitely-wrong")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(text, "join unavailable");
        assert!(!text.contains("definitely-wrong"));
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
    async fn join_redemption_is_rate_limited() {
        let auth = Arc::new(Auth::generate());
        let mut state = AppState::in_memory(auth.clone());
        state.join_redemption_rate = Arc::new(ratelimit::TokenBucket::new(0.0, 0.0));
        let session = "ABCDEF";
        state.store.ensure(session, "host").await.unwrap();
        let token = auth
            .mint_join_link(
                &crate::session_identity::SessionScope::for_session(session),
                std::time::Duration::from_secs(1800),
                std::time::SystemTime::now(),
            )
            .unwrap();
        let (status, headers, body) = join_redemption_response(
            state,
            Request::builder()
                .method("POST")
                .uri(format!("/join/{session}/participants"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Alice"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body, "join unavailable");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn join_redemption_returns_the_same_unavailable_response_for_all_unavailable_cases() {
        let auth = Arc::new(Auth::generate());
        let state = AppState::in_memory(auth.clone());
        state.store.ensure("ABCDEF", "host").await.unwrap();
        let now = std::time::SystemTime::now();
        let scope = crate::session_identity::SessionScope::for_session("ABCDEF");
        let other_scope = crate::session_identity::SessionScope::for_session("UVWXYZ");
        let valid = auth
            .mint_join_link(&scope, std::time::Duration::from_secs(1800), now)
            .unwrap();
        let expired = auth
            .mint_join_link(&scope, std::time::Duration::from_secs(0), now)
            .unwrap();
        let cross_scope = auth
            .mint_join_link(&other_scope, std::time::Duration::from_secs(1800), now)
            .unwrap();
        let mut tampered = valid.clone();
        tampered.pop();
        tampered.push(if valid.ends_with('A') { 'B' } else { 'A' });

        let expected = join_redemption_response(
            state.clone(),
            Request::builder()
                .method("POST")
                .uri("/join/ABCDEF/participants")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(expected.0, StatusCode::UNAUTHORIZED);
        assert_eq!(expected.2, "join unavailable");

        let cases = [
            (
                "malformed bearer",
                Request::builder()
                    .method("POST")
                    .uri("/join/ABCDEF/participants")
                    .header("authorization", "Bearer ")
                    .body(Body::empty())
                    .unwrap(),
            ),
            (
                "invalid code",
                Request::builder()
                    .method("POST")
                    .uri("/join/ABC!EF/participants")
                    .header("authorization", format!("Bearer {valid}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
            (
                "bad signature",
                Request::builder()
                    .method("POST")
                    .uri("/join/ABCDEF/participants")
                    .header("authorization", format!("Bearer {tampered}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
            (
                "expired",
                Request::builder()
                    .method("POST")
                    .uri("/join/ABCDEF/participants")
                    .header("authorization", format!("Bearer {expired}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
            (
                "cross scope",
                Request::builder()
                    .method("POST")
                    .uri("/join/ABCDEF/participants")
                    .header("authorization", format!("Bearer {cross_scope}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
            (
                "session absent",
                Request::builder()
                    .method("POST")
                    .uri("/join/NOPQRS/participants")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            auth.mint_join_link(
                                &crate::session_identity::SessionScope::for_session("NOPQRS"),
                                std::time::Duration::from_secs(1800),
                                now
                            )
                            .unwrap()
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            ),
        ];

        for (label, request) in cases {
            let actual = join_redemption_response(state.clone(), request).await;
            assert_eq!(actual.0, expected.0, "{label}");
            assert_eq!(actual.1, expected.1, "{label}");
            assert_eq!(actual.2, expected.2, "{label}");
        }
    }

    #[tokio::test]
    async fn join_redemption_handler_recheck_stays_non_enumerating() {
        let auth = Arc::new(Auth::generate());
        let mut state = AppState::in_memory(auth.clone());
        state.store = Arc::new(FlakyJoinStore::new());
        state.store.ensure("ABCDEF", "host").await.unwrap();
        let token = auth
            .mint_join_link(
                &crate::session_identity::SessionScope::for_session("ABCDEF"),
                std::time::Duration::from_secs(1800),
                std::time::SystemTime::now(),
            )
            .unwrap();
        let actual = join_redemption_response(
            state,
            Request::builder()
                .method("POST")
                .uri("/join/ABCDEF/participants")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Alice"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(actual.0, StatusCode::UNAUTHORIZED);
        assert_eq!(actual.2, "join unavailable");
    }

    #[tokio::test]
    async fn join_redemption_backend_failures_are_generic() {
        let auth = Arc::new(Auth::generate());
        let mut state = AppState::in_memory(auth.clone());
        state.store = Arc::new(ErrorJoinStore);
        let token = auth
            .mint_join_link(
                &crate::session_identity::SessionScope::for_session("ABCDEF"),
                std::time::Duration::from_secs(1800),
                std::time::SystemTime::now(),
            )
            .unwrap();
        let actual = join_redemption_response(
            state,
            Request::builder()
                .method("POST")
                .uri("/join/ABCDEF/participants")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(actual.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(actual.2, "join unavailable");
        assert_eq!(actual.1[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn join_redemption_applies_backpressure_at_the_concurrency_limit() {
        let auth = Arc::new(Auth::generate());
        let started = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        let release = Arc::new(Notify::new());
        let blocking = Arc::new(BlockingJoinStore::new(
            started.clone(),
            released.clone(),
            release.clone(),
        ));
        blocking.inner.ensure("ABCDEF", "host").await.unwrap();

        let mut state = AppState::in_memory(auth.clone());
        state.store = blocking.clone();
        state.join_redemption_rate = Arc::new(crate::ratelimit::TokenBucket::new(1000.0, 1000.0));
        let router = app(state);
        let token = auth
            .mint_join_link(
                &crate::session_identity::SessionScope::for_session("ABCDEF"),
                std::time::Duration::from_secs(1800),
                std::time::SystemTime::now(),
            )
            .unwrap();

        let mut handles = Vec::new();
        for i in 0..8 {
            let router = router.clone();
            let token = token.clone();
            handles.push(tokio::spawn(async move {
                router
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/join/ABCDEF/participants")
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .body(Body::from(format!(r#"{{"name":"p{i}"}}"#)))
                            .unwrap(),
                    )
                    .await
                    .unwrap()
                    .status()
            }));
        }

        tokio::time::timeout(Duration::from_secs(5), async {
            while started.load(Ordering::SeqCst) < 8 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let ninth_router = router.clone();
        let ninth_token = token.clone();
        let ninth = tokio::spawn(async move {
            ninth_router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/join/ABCDEF/participants")
                        .header("authorization", format!("Bearer {ninth_token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"name":"p9"}"#))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(started.load(Ordering::SeqCst), 8);
        assert!(
            !ninth.is_finished(),
            "the 9th request must wait for a permit"
        );

        released.store(true, Ordering::SeqCst);
        release.notify_waiters();
        assert_eq!(ninth.await.unwrap(), StatusCode::OK);
        for handle in handles {
            assert_eq!(handle.await.unwrap(), StatusCode::OK);
        }
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
