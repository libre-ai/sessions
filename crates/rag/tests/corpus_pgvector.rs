//! Ingestion + retrieval over pgvector. Requires a Postgres with the `vector`
//! extension; ignored by default. Run with:
//!
//! ```text
//! docker run --rm -d -p 5439:5432 -e POSTGRES_PASSWORD=presto --name presto-pgv pgvector/pgvector:pg16
//! DATABASE_URL=postgres://postgres:presto@127.0.0.1:5439/postgres \
//!   cargo test -p presto-rag --test corpus_pgvector -- --ignored --nocapture
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use presto_rag::corpus::{CorpusStore, Retriever};
use presto_rag::provider::FakeAiProvider;

#[tokio::test]
#[ignore = "requires DATABASE_URL with pgvector; see module docs"]
async fn ingest_then_retrieve_ranks_by_similarity() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL (pgvector) to run");
        return;
    };

    let store = CorpusStore::connect(&url).await.expect("connect");
    let provider = FakeAiProvider;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let doc = format!("doc-{nanos}");

    let text = "Paris is the capital of France.\n\n\
                The mitochondrion is the powerhouse of the cell.\n\n\
                Rust enforces memory safety without a garbage collector.";
    let stored = store.ingest(&doc, text, &provider).await.expect("ingest");
    assert_eq!(stored, 3);

    // A query equal to the third chunk's text yields an identical (fake)
    // embedding, so that chunk ranks first at distance ~0.
    let results = store
        .retrieve(
            "Rust enforces memory safety without a garbage collector.",
            1,
            &provider,
        )
        .await
        .expect("retrieve");
    assert_eq!(results.len(), 1);
    assert!(
        results[0].text.contains("Rust"),
        "expected the Rust chunk, got {:?}",
        results[0]
    );
    assert!(
        results[0].distance < 1e-4,
        "exact-text match should be distance ~0, got {}",
        results[0].distance
    );
    eprintln!(
        "ingest+retrieve over pgvector: exact-text query returned its chunk (distance {})",
        results[0].distance
    );
}
