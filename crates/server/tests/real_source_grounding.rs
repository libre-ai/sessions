//! Integration test: verify real source ingestion and grounded questions.
//! This test ensures that questions can cite verifiable sources from gear-loader.

#[tokio::test]
#[ignore] // Requires live Postgres; run with: cargo test -- --ignored
async fn test_markdown_ingestion_creates_source_ref() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL not set");
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    // Schema is programmatic and idempotent, like the other stores in this
    // repo (no migrations directory). ingest_markdown ensures it too; calling
    // it here keeps the test meaningful if that internal call ever moves.
    presto_server::ingestion::ensure_schema(&pool)
        .await
        .expect("schema init failed");

    let markdown_content = r#"# Test Document

This is a test document for grounding.

## Section 1

Some important content here.
"#;

    let result =
        presto_server::ingestion::ingest_markdown(markdown_content, "test-doc.md", &pool).await;

    assert!(result.is_ok(), "ingestion should succeed");
    let source_ref = result.unwrap();

    assert!(!source_ref.source_id.is_empty());
    assert_eq!(source_ref.origin_product, "rumble-lm");
    assert!(source_ref.canonical_text.is_some());
    assert!(
        source_ref
            .canonical_text
            .as_ref()
            .map(|t| t.contains("Test Document"))
            .unwrap_or(false)
    );

    // Verify persistence: retrieve by ID
    let retrieved = presto_server::ingestion::SourceRef::get_by_id(&pool, &source_ref.source_id)
        .await
        .expect("get_by_id should not error")
        .expect("source_ref should be persisted");

    assert_eq!(retrieved.source_id, source_ref.source_id);
    assert_eq!(retrieved.origin_product, source_ref.origin_product);
}

#[test]
fn test_grounded_quiz_has_verified_citations() {
    use presto_core::protocol::CitationValidationStatus;

    let quiz = presto_server::grounded_fixtures::grounded_quiz("src_rust_ownership_test");

    // Verify all grounded questions are marked as verified, not fixture
    for question in &quiz {
        let citation_validation = question
            .citation_validation
            .as_ref()
            .expect("citation_validation must be set");

        assert_eq!(
            citation_validation.status,
            CitationValidationStatus::Verified,
            "grounded question {} must be verified, not fixture",
            question.id
        );
        assert!(
            citation_validation.citation_count >= 1,
            "grounded question {} must cite at least 1 source",
            question.id
        );
        assert!(
            !question.source_section_ids.is_empty(),
            "grounded question {} must have source section IDs",
            question.id
        );
    }
}

#[test]
fn test_grounded_quiz_structure_valid() {
    let quiz = presto_server::grounded_fixtures::grounded_quiz("src_rust_ownership");

    assert!(!quiz.is_empty(), "grounded quiz must have questions");
    for question in &quiz {
        assert!(!question.text.is_empty());
        assert!(!question.choices.is_empty());
        assert!(!question.correct_choices.is_empty());
        // All correct_choices must be within range
        assert!(
            question
                .correct_choices
                .iter()
                .all(|&c| (c as usize) < question.choices.len()),
            "correct_choices must be in range for {}",
            question.id
        );
        // All source_section_ids must be non-empty
        assert!(
            question.source_section_ids.iter().all(|s| !s.is_empty()),
            "source_section_ids must be non-empty for {}",
            question.id
        );
    }
}

#[test]
fn test_gear_loader_contract_shape() {
    // Verify gear-loader's CanonicalSourceDocument API is available
    let _csd_format = gear_loader::CANONICAL_SOURCE_DOCUMENT_FORMAT;
    let _extraction_format = gear_loader::EXTRACTION_REQUEST_FORMAT;

    // These should be accessible without error
    assert_eq!(_csd_format, "wrench.canonical_source_document.v0.1");
}
