//! presto-server — thin binary entry point. Builds the app state from the
//! environment (single-instance by default, multi-instance when Postgres/Redis/a
//! shared Biscuit key are configured) and serves it.
//!
//! `presto-server keygen` prints a fresh hex Ed25519 private key to set as
//! `BISCUIT_PRIVATE_KEY` (required, and shared, for multi-instance deployments).

use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use presto_rag::corpus::{CorpusStore, Retriever};
use presto_rag::provider::{AiProvider, OpenAiCompatible};
use presto_server::auth::Auth;
use presto_server::fanout::{BroadcastFanout, Fanout};
use presto_server::membership::{InMemoryMembershipStore, MembershipStore};
use presto_server::oidc::OidcConfig;
use presto_server::owner_auth::{OwnerAuth, OwnerAuthConfig};
use presto_server::postgres_store::PostgresSessionStore;
use presto_server::quiz::{
    BreakoutSource, DocumentIngestor, FixtureBreakoutSource, FixtureFlashcardSource,
    FixtureIngestor, FixtureQuizSource, FlashcardSource, GroundedQuizSource, QuizSource,
    RagBreakoutSource, RagFlashcardSource, RagIngestor, RagQuizSource,
};
use presto_server::ratelimit::TokenBucket;
use presto_server::redis_fanout::RedisFanout;
use presto_server::store::{InMemorySessionStore, SessionStore};
use presto_server::{AppState, app};

/// The token authority: a shared key from `BISCUIT_PRIVATE_KEY` (required for
/// multi-instance), or an ephemeral key for single-instance/local runs.
fn build_auth() -> Result<Arc<Auth>, Box<dyn Error>> {
    match std::env::var("BISCUIT_PRIVATE_KEY") {
        Ok(hex) => {
            println!("biscuit: shared key from BISCUIT_PRIVATE_KEY");
            Ok(Arc::new(Auth::from_private_key_hex(&hex)?))
        }
        Err(_) => {
            eprintln!(
                "biscuit: BISCUIT_PRIVATE_KEY unset — using an ephemeral key (single instance \
                 only; run `presto-server keygen` to mint a shared key)"
            );
            Ok(Arc::new(Auth::generate()))
        }
    }
}

/// Shared Postgres state when `DATABASE_URL` is set (fails loudly if it is set
/// but unreachable — never silently falls back, which would split state); else
/// in-memory.
async fn build_store() -> Result<Arc<dyn SessionStore>, Box<dyn Error>> {
    match std::env::var("DATABASE_URL") {
        Ok(url) => {
            println!("session store: Postgres (shared, multi-instance)");
            Ok(Arc::new(PostgresSessionStore::connect(&url).await?))
        }
        Err(_) => {
            println!("session store: in-memory (single instance)");
            Ok(Arc::new(InMemorySessionStore::new()))
        }
    }
}

/// Redis fanout when `REDIS_URL` is set (fails loudly if unreachable); else an
/// in-process broadcast.
async fn build_fanout() -> Result<Arc<dyn Fanout>, Box<dyn Error>> {
    match std::env::var("REDIS_URL") {
        Ok(url) => {
            println!("fanout: Redis (multi-instance)");
            Ok(Arc::new(RedisFanout::connect(&url).await?))
        }
        Err(_) => {
            println!("fanout: in-process broadcast (single instance)");
            Ok(Arc::new(BroadcastFanout::new()))
        }
    }
}

/// RAG pipeline quiz when a corpus database and an AI provider are configured;
/// otherwise the fixture quiz.
/// Quiz + breakout content from the RAG pipeline when a corpus database and an
/// AI provider are configured (sharing one corpus + provider); otherwise the
/// fixture sources.
type Content = (
    Arc<dyn QuizSource>,
    Arc<dyn BreakoutSource>,
    Arc<dyn FlashcardSource>,
    Arc<dyn DocumentIngestor>,
);

async fn build_content(_store: &Arc<dyn SessionStore>) -> Content {
    let fixture: Content = (
        Arc::new(FixtureQuizSource),
        Arc::new(FixtureBreakoutSource),
        Arc::new(FixtureFlashcardSource),
        Arc::new(FixtureIngestor),
    );

    // Try RAG pipeline first. Hosted routing is Clever AI only; local
    // development must opt into a loopback endpoint explicitly.
    let provider = if std::env::var("LOCAL_AI_ENABLED").as_deref() == Ok("1") {
        OpenAiCompatible::from_local_env()
    } else {
        OpenAiCompatible::from_env()
    };
    let (Ok(database_url), Ok(provider)) = (std::env::var("DATABASE_URL"), provider) else {
        // No RAG pipeline: try grounded quiz (real sources)
        println!("content: grounded quiz (real ingested sources)");

        // Attempt to ingest and initialize grounded sources from gear-memory FileStore
        if let Ok(_url) = std::env::var("DATABASE_URL") {
            // Set up gear-memory FileStore (path from env or default)
            let gear_memory_path = std::env::var("GEAR_MEMORY_STORE")
                .unwrap_or_else(|_| ".gear_memory_sources".to_string());
            let file_store_result =
                gear_memory::FileStore::new(std::path::Path::new(&gear_memory_path));

            match file_store_result {
                Ok(file_store) => {
                    // Initialize sources and return grounded quiz
                    match presto_server::grounded_fixtures::initialize_sources(&file_store).await {
                        Ok(sources) => {
                            if let Some(src) = sources.first() {
                                println!(
                                    "content: grounded quiz initialized with source {} ({})",
                                    src.source_id,
                                    src.canonical_title.as_deref().unwrap_or("untitled")
                                );
                                return (
                                    Arc::new(GroundedQuizSource::new(&src.source_id)),
                                    Arc::new(FixtureBreakoutSource),
                                    Arc::new(FixtureFlashcardSource),
                                    Arc::new(FixtureIngestor),
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "grounded source initialization failed ({e}); falling back to fixture"
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "gear-memory FileStore creation failed ({e}); falling back to fixture"
                    );
                }
            }
        }

        // Fallback to fixture
        println!(
            "content: fixture (configure DATABASE_URL plus loopback local AI or enabled Clever AI for RAG)"
        );
        return fixture;
    };

    match CorpusStore::connect(&database_url).await {
        Ok(corpus) => {
            println!(
                "content: RAG quiz + breakout + flashcards + ingestion (pgvector corpus + AI provider)"
            );
            // Keep the concrete corpus for ingestion (which is not on the
            // read-only `Retriever` seam), and reuse it as the retriever.
            let corpus = Arc::new(corpus);
            let provider: Arc<dyn AiProvider> = Arc::new(provider);
            let retriever: Arc<dyn Retriever> = corpus.clone();
            (
                Arc::new(RagQuizSource::new(retriever.clone(), provider.clone())),
                Arc::new(RagBreakoutSource::new(retriever.clone(), provider.clone())),
                Arc::new(RagFlashcardSource::new(retriever, provider.clone())),
                Arc::new(RagIngestor::new(corpus, provider)),
            )
        }
        Err(e) => {
            eprintln!("corpus unavailable ({e}); falling back to fixture content");
            fixture
        }
    }
}

enum OwnerAuthEnvironment {
    Disabled,
    Enabled(OidcConfig),
}

fn owner_auth_environment() -> Result<OwnerAuthEnvironment, Box<dyn Error>> {
    let issuer = std::env::var("OIDC_ISSUER").ok();
    let client_id = std::env::var("OIDC_CLIENT_ID").ok();
    let redirect_uri = std::env::var("OIDC_REDIRECT_URI").ok();
    match (issuer, client_id, redirect_uri) {
        (None, None, None) => Ok(OwnerAuthEnvironment::Disabled),
        (Some(issuer), Some(client_id), Some(redirect_uri)) => Ok(OwnerAuthEnvironment::Enabled(
            OidcConfig::new(issuer, client_id, redirect_uri),
        )),
        _ => Err("OIDC_ISSUER, OIDC_CLIENT_ID and OIDC_REDIRECT_URI must be set together".into()),
    }
}

fn validate_owner_auth_topology(
    enabled: bool,
    explicit_single_instance: Option<&str>,
    database_configured: bool,
    redis_configured: bool,
) -> Result<(), &'static str> {
    if !enabled {
        return Ok(());
    }
    if explicit_single_instance != Some("1") {
        return Err("owner auth requires OWNER_AUTH_SINGLE_INSTANCE=1");
    }
    if database_configured || redis_configured {
        return Err(
            "owner auth is process-local and cannot start with DATABASE_URL or REDIS_URL configured",
        );
    }
    Ok(())
}

/// Enable owner auth only when the complete OIDC tuple and the explicit
/// single-instance topology gate are configured. Discovery is performed at
/// startup and fails closed; no silent local-auth fallback.
async fn build_owner_auth(
    auth: Arc<Auth>,
    environment: OwnerAuthEnvironment,
) -> Result<Arc<OwnerAuth>, Box<dyn Error>> {
    match environment {
        OwnerAuthEnvironment::Disabled => {
            println!("owner auth: disabled (OIDC_* variables unset)");
            Ok(Arc::new(OwnerAuth::disabled(auth)))
        }
        OwnerAuthEnvironment::Enabled(oidc) => {
            let config = OwnerAuthConfig::new(oidc)?;
            let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
            eprintln!(
                "owner auth: explicitly single-instance, with process-local sessions/membership; restart logs users out"
            );
            Ok(Arc::new(
                OwnerAuth::discover(config, auth, membership).await?,
            ))
        }
    }
}

/// The `POST /sessions` rate limiter: burst + steady refill, tunable via
/// `SESSION_RATE_BURST` and `SESSION_RATE_PER_SEC` (defaults 30 burst, 1/sec).
fn build_session_rate() -> Arc<TokenBucket> {
    let burst = std::env::var("SESSION_RATE_BURST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let per_sec = std::env::var("SESSION_RATE_PER_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    Arc::new(TokenBucket::new(burst, per_sec))
}

/// The private join-link redemption limiter: separate bucket, defaults 60 burst
/// and 2 redemptions/sec; tune via `JOIN_RATE_BURST` and `JOIN_RATE_PER_SEC`.
fn build_join_redemption_rate() -> Arc<TokenBucket> {
    let burst = std::env::var("JOIN_RATE_BURST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60.0);
    let per_sec = std::env::var("JOIN_RATE_PER_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0);
    Arc::new(TokenBucket::new(burst, per_sec))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();

    // `presto-server keygen` mints a shared Biscuit private key and exits.
    if std::env::args().nth(1).as_deref() == Some("keygen") {
        println!("{}", Auth::generate().private_key_hex());
        return Ok(());
    }

    let owner_auth_environment = owner_auth_environment()?;
    validate_owner_auth_topology(
        matches!(&owner_auth_environment, OwnerAuthEnvironment::Enabled(_)),
        std::env::var("OWNER_AUTH_SINGLE_INSTANCE").ok().as_deref(),
        std::env::var_os("DATABASE_URL").is_some(),
        std::env::var_os("REDIS_URL").is_some(),
    )?;

    let legacy_ingest_token = std::env::var("INGEST_TOKEN")
        .map_err(|_| "INGEST_TOKEN is required for the legacy ingestion route")?;
    if !presto_server::http::validate_legacy_ingest_token(&legacy_ingest_token) {
        return Err("INGEST_TOKEN must be 32-512 printable ASCII bytes (0x21-0x7e)".into());
    }
    let legacy_ingest_token: Arc<str> = Arc::from(legacy_ingest_token);

    let store = build_store().await?;
    let (quiz, breakout, flashcards, ingestor) = build_content(&store).await;
    let auth = build_auth()?;
    let owner_auth = build_owner_auth(auth.clone(), owner_auth_environment).await?;
    let owner_corpus = Arc::new(presto_server::owner_corpus::OwnerCorpusStore::new());
    let state = AppState {
        store,
        fanout: build_fanout().await?,
        auth,
        owner_auth,
        owner_corpus: owner_corpus.clone(),
        approved_claims: Arc::new(
            presto_server::approved_claims::ApprovedClaimRegistry::with_owner_corpus(
                owner_corpus.clone(),
            ),
        ),
        notebook_rag: Arc::new(
            presto_server::notebook_rag::StagedNotebookRagEngine::fixture_with_owner_corpus(
                owner_corpus,
            ),
        ),
        quiz,
        breakout,
        flashcards,
        ingestor,
        legacy_ingest_token: Some(legacy_ingest_token),
        session_rate: build_session_rate(),
        join_redemption_rate: build_join_redemption_rate(),
    };

    // Clever Cloud injects `PORT`; default to 8080 for local runs.
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("presto-server listening on {addr}");
    axum::serve(listener, app(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_auth_requires_explicit_single_instance_mode() {
        assert_eq!(
            validate_owner_auth_topology(true, None, false, false),
            Err("owner auth requires OWNER_AUTH_SINGLE_INSTANCE=1")
        );
        assert_eq!(
            validate_owner_auth_topology(true, Some("true"), false, false),
            Err("owner auth requires OWNER_AUTH_SINGLE_INSTANCE=1")
        );
        assert!(validate_owner_auth_topology(true, Some("1"), false, false).is_ok());
    }

    #[test]
    fn owner_auth_refuses_known_distributed_configurations() {
        for (database, redis) in [(true, false), (false, true), (true, true)] {
            assert_eq!(
                validate_owner_auth_topology(true, Some("1"), database, redis),
                Err(
                    "owner auth is process-local and cannot start with DATABASE_URL or REDIS_URL configured"
                )
            );
        }
        // Live anonymous routes retain their existing distributed adapters when
        // owner auth is disabled.
        assert!(validate_owner_auth_topology(false, None, true, true).is_ok());
    }
}
