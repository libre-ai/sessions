//! presto-server — thin binary entry point. Builds the app state from the
//! environment (single-instance by default, multi-instance when Postgres/Redis/a
//! shared Biscuit key are configured) and serves it.
//!
//! `presto-server keygen` prints a fresh hex Ed25519 private key to set as
//! `BISCUIT_PRIVATE_KEY` (required, and shared, for multi-instance deployments).

use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use presto_rag::corpus::CorpusStore;
use presto_rag::provider::OpenAiCompatible;
use presto_server::auth::Auth;
use presto_server::fanout::{BroadcastFanout, Fanout};
use presto_server::postgres_store::PostgresSessionStore;
use presto_server::quiz::{FixtureQuizSource, QuizSource, RagQuizSource};
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
async fn build_quiz() -> Arc<dyn QuizSource> {
    let (Ok(database_url), Ok(provider)) =
        (std::env::var("DATABASE_URL"), OpenAiCompatible::from_env())
    else {
        println!("quiz source: fixture (set DATABASE_URL + AI_BASE_URL + AI_API_KEY for RAG)");
        return Arc::new(FixtureQuizSource);
    };
    match CorpusStore::connect(&database_url).await {
        Ok(corpus) => {
            println!("quiz source: RAG (pgvector corpus + AI provider)");
            Arc::new(RagQuizSource::new(Arc::new(corpus), Arc::new(provider)))
        }
        Err(e) => {
            eprintln!("corpus unavailable ({e}); falling back to fixture quiz");
            Arc::new(FixtureQuizSource)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // `presto-server keygen` mints a shared Biscuit private key and exits.
    if std::env::args().nth(1).as_deref() == Some("keygen") {
        println!("{}", Auth::generate().private_key_hex());
        return Ok(());
    }

    let state = AppState {
        store: build_store().await?,
        fanout: build_fanout().await?,
        auth: build_auth()?,
        quiz: build_quiz().await,
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
