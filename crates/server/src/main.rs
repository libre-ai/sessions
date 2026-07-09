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

async fn build_content(
    _store: &Arc<dyn SessionStore>,
) -> Content {
    let fixture: Content = (
        Arc::new(FixtureQuizSource),
        Arc::new(FixtureBreakoutSource),
        Arc::new(FixtureFlashcardSource),
        Arc::new(FixtureIngestor),
    );

    // Try RAG pipeline first
    let (Ok(database_url), Ok(provider)) =
        (std::env::var("DATABASE_URL"), OpenAiCompatible::from_env())
    else {
        // No RAG pipeline: try grounded quiz (real sources)
        println!("content: grounded quiz (real ingested sources)");

        // Attempt to ingest and initialize grounded sources
        // Extract PgPool if available (Postgres store)
        if let Ok(url) = std::env::var("DATABASE_URL") {
            if let Ok(pool) = sqlx::PgPool::connect(&url).await {
                // Initialize sources and return grounded quiz
                match presto_server::grounded_fixtures::initialize_sources(&pool).await {
                    Ok(sources) => {
                        if let Some(src) = sources.first() {
                            println!(
                                "content: grounded quiz initialized with source {} ({})",
                                src.source_id, src.canonical_title.as_deref().unwrap_or("untitled")
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
                        eprintln!("grounded source initialization failed ({e}); falling back to fixture");
                    }
                }
            }
        }

        // Fallback to fixture
        println!("content: fixture (set DATABASE_URL + AI_BASE_URL + AI_API_KEY for RAG)");
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // `presto-server keygen` mints a shared Biscuit private key and exits.
    if std::env::args().nth(1).as_deref() == Some("keygen") {
        println!("{}", Auth::generate().private_key_hex());
        return Ok(());
    }

    let store = build_store().await?;
    let (quiz, breakout, flashcards, ingestor) = build_content(&store).await;
    let state = AppState {
        store,
        fanout: build_fanout().await?,
        auth: build_auth()?,
        quiz,
        breakout,
        flashcards,
        ingestor,
        session_rate: build_session_rate(),
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
