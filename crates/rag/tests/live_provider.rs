//! Live proof against a real OpenAI-compatible endpoint: embeddings have a
//! consistent non-empty dimension, generation returns a parseable in-range
//! question, and verification returns a parseable verdict. The value of a real
//! model's verdict is model-dependent, so only parseability is asserted.
//!
//! Ignored by default. Local runs are loopback-only. The only hosted route is
//! explicitly enabled Clever AI with a versioned contract reference.
//!
//! The base URL is the server **origin without `/v1`** — the client appends
//! `/v1/chat/completions` and `/v1/embeddings` itself.
//!
//! Some local runtimes reject `response_format: json_object`; set
//! `LOCAL_AI_JSON_MODE=0` and the pipeline parses the JSON
//! out of the plain-text reply. Use `127.0.0.1` rather than `localhost` — LM
//! Studio binds IPv4 only, and `localhost` may resolve to `::1` first.
//!
//! ```text
//! # LM Studio (local): in the app, load BOTH an embedding model and a chat
//! #   model, then Developer -> Start Server (default port 1234). The model ids
//! #   must match what is loaded (check GET http://127.0.0.1:1234/v1/models).
//! LOCAL_AI_ENABLED=1 LOCAL_AI_BASE_URL=http://127.0.0.1:1234 \
//!   LOCAL_AI_JSON_MODE=0 LOCAL_AI_EMBED_MODEL=<loaded-embedding-model> \
//!   LOCAL_AI_CHAT_MODEL=<loaded-chat-model> \
//!   cargo test -p presto-rag --test live_provider -- --ignored --nocapture
//!
//! # Hosted Clever AI requires CLEVER_AI_ENABLED=1 plus endpoint, models,
//! # credential and CLEVER_AI_CONTRACT_REF. Do not run without contract approval.
//! ```

use presto_rag::corpus::Chunk;
use presto_rag::generate::generate_from_chunk;
use presto_rag::provider::{AiProvider, OpenAiCompatible};
use presto_rag::verify::verify_grounding;

#[tokio::test]
#[ignore = "requires an explicitly enabled loopback or Clever AI route; see module docs"]
async fn real_provider_embeds_generates_and_verifies() {
    let provider = if std::env::var("LOCAL_AI_ENABLED").as_deref() == Ok("1") {
        OpenAiCompatible::from_local_env()
    } else {
        OpenAiCompatible::from_env()
    };
    let Ok(provider) = provider else {
        eprintln!("skipping: no approved AI route is enabled");
        return;
    };

    // Embeddings: consistent, non-empty dimensions.
    let vecs = provider
        .embed(&[
            "the sky is blue".into(),
            "rust is a systems language".into(),
        ])
        .await
        .expect("embed call failed");
    assert_eq!(vecs.len(), 2);
    assert!(!vecs[0].is_empty(), "embeddings must be non-empty");
    assert_eq!(vecs[0].len(), vecs[1].len(), "dimension must be consistent");

    // Generation: a parseable, in-range question grounded in the source.
    let chunk = Chunk {
        source_section_id: "doc#p0".into(),
        text: "The Eiffel Tower is a wrought-iron lattice tower in Paris, completed in 1889."
            .into(),
    };
    let q = generate_from_chunk(&chunk, &provider)
        .await
        .expect("generation failed");
    assert!(q.choices.len() >= 2, "a question needs choices");
    assert!(
        !q.correct_choices.is_empty(),
        "a question needs a correct answer"
    );
    assert!(
        q.correct_choices
            .iter()
            .all(|&c| (c as usize) < q.choices.len()),
        "every correct_choice must index a real option"
    );
    assert_eq!(q.source_section_ids, vec!["doc#p0".to_string()]);

    // Verification: a parseable verdict (the boolean is model-dependent).
    let verdict = verify_grounding(&q, &chunk, &provider)
        .await
        .expect("verification failed");
    eprintln!(
        "real provider OK: dim={} | generated question parsed | evidence_validated={}",
        vecs[0].len(),
        verdict.is_supported(),
    );
}
