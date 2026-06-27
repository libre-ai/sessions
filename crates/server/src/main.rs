//! presto-server — thin binary entry point. Builds the app state and serves it.

use std::net::SocketAddr;
use std::sync::Arc;

use presto_rag::corpus::CorpusStore;
use presto_rag::provider::OpenAiCompatible;
use presto_server::auth::Auth;
use presto_server::fanout::BroadcastFanout;
use presto_server::quiz::{FixtureQuizSource, QuizSource, RagQuizSource};
use presto_server::store::InMemorySessionStore;
use presto_server::{AppState, app};

/// Use the RAG pipeline (retrieve → generate → verify) when a corpus database and
/// an AI provider are configured; otherwise fall back to the fixture quiz.
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TB-1/2 run single-instance (in-memory). A multi-instance deployment swaps
    // in the Redis fanout + Postgres store behind the same `AppState` seams.
    // The Biscuit key is ephemeral here; production loads `BISCUIT_PRIVATE_KEY`.
    let state = AppState {
        store: Arc::new(InMemorySessionStore::new()),
        fanout: Arc::new(BroadcastFanout::new()),
        auth: Arc::new(Auth::generate()),
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
