//! Ingestion + retrieval over pgvector. Requires a Postgres with the `vector`
//! extension; ignored by default. Run with:
//!
//! ```text
//! docker run --rm -d -p 5439:5432 -e POSTGRES_PASSWORD=presto --name presto-pgv pgvector/pgvector:pg16
//! DATABASE_URL=postgres://postgres:presto@127.0.0.1:5439/postgres \
//!   cargo test -p presto-rag --test corpus_pgvector -- --ignored --nocapture
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use presto_rag::corpus::{CorpusStore, RetrievalScope, Retriever};
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
    let stored = store
        .ingest("default", 0, &doc, text, &provider)
        .await
        .expect("ingest");
    assert_eq!(stored, 3);

    // A query equal to the third chunk's text yields an identical (fake)
    // embedding, so that chunk ranks first at distance ~0.
    let results = store
        .retrieve(
            &RetrievalScope::wedge(),
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

/// Retrieval never crosses `space_id` nor returns chunks above the requester's
/// cleared confidentiality — the §1 moat KPIs for cross-tenant + clearance
/// isolation. The three docs share identical text (hence identical fake
/// embeddings), so only the scope filter can keep the wrong ones out.
#[tokio::test]
#[ignore = "requires DATABASE_URL with pgvector; see module docs"]
async fn retrieve_never_crosses_space_or_clearance() {
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
    let (sa, sb) = (format!("space-A-{nanos}"), format!("space-B-{nanos}"));
    let shared = "Shared topic about quantum entanglement and teleportation.";

    // Same text in space A (public), space B (public), and space A (confidential).
    store
        .ingest(&sa, 0, "a-pub", shared, &provider)
        .await
        .expect("ingest a-pub");
    store
        .ingest(&sb, 0, "b-pub", shared, &provider)
        .await
        .expect("ingest b-pub");
    store
        .ingest(&sa, 2, "a-secret", shared, &provider)
        .await
        .expect("ingest a-secret");

    // Requester is in space A, cleared to level 1.
    let scope = RetrievalScope {
        space_id: sa.clone(),
        max_confidentiality: 1,
    };
    let hits = store
        .retrieve(&scope, shared, 10, &provider)
        .await
        .expect("retrieve");

    // Only a-pub#p0 survives: space B is another space; a-secret is over clearance.
    assert_eq!(
        hits.len(),
        1,
        "exactly one in-scope chunk expected, got {:?}",
        hits.iter()
            .map(|h| &h.source_section_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].source_section_id, "a-pub#p0");

    // fetch_section honours the same scope.
    assert!(
        store
            .fetch_section(&scope, "a-secret#p0")
            .await
            .unwrap()
            .is_none(),
        "a section above clearance must be invisible"
    );
    let cross = RetrievalScope {
        space_id: sb.clone(),
        max_confidentiality: i16::MAX,
    };
    assert!(
        store
            .fetch_section(&cross, "a-pub#p0")
            .await
            .unwrap()
            .is_none(),
        "a section in another space must be invisible"
    );
    assert!(
        store
            .fetch_section(&scope, "a-pub#p0")
            .await
            .unwrap()
            .is_some(),
        "the in-scope section must be visible"
    );

    eprintln!("retrieval honoured space + clearance: 1/3 same-text chunks returned");
}
