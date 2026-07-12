//! Gated end-to-end ingestion: `POST /corpus/documents` → `RagIngestor` →
//! `CorpusStore` (pgvector). Requires a Postgres with the `vector` extension;
//! ignored by default. Run with:
//!
//! ```text
//! docker run --rm -d -p 5439:5432 -e POSTGRES_PASSWORD=presto --name presto-pgv pgvector/pgvector:pg16
//! DATABASE_URL=postgres://postgres:presto@127.0.0.1:5439/postgres \
//!   cargo test -p presto-server --test ingest -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use presto_rag::corpus::CorpusStore;
use presto_rag::provider::FakeAiProvider;
use presto_server::auth::Auth;
use presto_server::quiz::RagIngestor;
use presto_server::{AppState, app};

#[tokio::test]
#[ignore = "requires DATABASE_URL with pgvector; see module docs"]
async fn http_ingest_stores_chunks_in_the_corpus() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL (pgvector) to run");
        return;
    };

    let corpus = Arc::new(CorpusStore::connect(&url).await.expect("connect"));
    let provider = Arc::new(FakeAiProvider);
    let mut state = AppState::in_memory(Arc::new(Auth::generate()));
    state.ingestor = Arc::new(RagIngestor::new(corpus, provider));
    state.legacy_ingest_token = Some(Arc::from("0123456789abcdef0123456789abcdef"));

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let doc = format!("http-doc-{nanos}");

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/corpus/documents?document_id={doc}"))
                .header("content-type", "text/markdown; charset=utf-8")
                .header("authorization", "Bearer 0123456789abcdef0123456789abcdef")
                .body(Body::from(
                    "Paris is the capital of France.\n\nRust is memory safe.",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    // Two paragraphs → two chunks stored.
    assert_eq!(v["data"]["chunks_stored"], 2);
    assert_eq!(v["data"]["document_id"], doc);
    eprintln!(
        "HTTP ingest stored {} chunks for {doc} ✅",
        v["data"]["chunks_stored"]
    );
}
