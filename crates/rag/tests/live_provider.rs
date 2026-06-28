//! Live proof against a real OpenAI-compatible endpoint: embeddings have a
//! consistent non-empty dimension, generation returns a parseable in-range
//! question, and verification returns a parseable verdict. The value of a real
//! model's verdict is model-dependent, so only parseability is asserted.
//!
//! Ignored by default. The endpoint is vendor-agnostic — run against any
//! OpenAI-compatible server. Local default: LM Studio. Hosted sovereign default:
//! Mistral (Paris); Clever AI is not yet GA (private alpha as of 2026-06).
//!
//! `AI_BASE_URL` is the server **origin without `/v1`** — the client appends
//! `/v1/chat/completions` and `/v1/embeddings` itself.
//!
//! Some local runtimes (e.g. Gemma on LM Studio) reject `response_format:
//! json_object` with a 400; set `AI_JSON_MODE=0` and the pipeline parses the JSON
//! out of the plain-text reply. Use `127.0.0.1` rather than `localhost` — LM
//! Studio binds IPv4 only, and `localhost` may resolve to `::1` first.
//!
//! ```text
//! # LM Studio (local): in the app, load BOTH an embedding model and a chat
//! #   model, then Developer -> Start Server (default port 1234). The model ids
//! #   must match what is loaded (check GET http://127.0.0.1:1234/v1/models).
//! AI_BASE_URL=http://127.0.0.1:1234 AI_API_KEY=lm-studio AI_JSON_MODE=0 \
//!   AI_EMBED_MODEL=text-embedding-nomic-embed-text-v1.5 \
//!   AI_CHAT_MODEL=google/gemma-4-12b-qat \
//!   cargo test -p presto-rag --test live_provider -- --ignored --nocapture
//! # Verified 2026-06-28: nomic-embed v1.5 (dim 768) + gemma-4-12b-qat,
//! #   embed + generate + grounding-verify all green.
//!
//! # Mistral (Paris, hosted sovereign):
//! AI_BASE_URL=https://api.mistral.ai AI_API_KEY=$MISTRAL_KEY \
//!   AI_EMBED_MODEL=mistral-embed AI_CHAT_MODEL=mistral-small-latest \
//!   cargo test -p presto-rag --test live_provider -- --ignored --nocapture
//! ```

use presto_rag::corpus::Chunk;
use presto_rag::generate::generate_from_chunk;
use presto_rag::provider::{AiProvider, OpenAiCompatible};
use presto_rag::verify::verify_grounding;

#[tokio::test]
#[ignore = "requires AI_BASE_URL + AI_API_KEY; see module docs"]
async fn real_provider_embeds_generates_and_verifies() {
    let Ok(provider) = OpenAiCompatible::from_env() else {
        eprintln!("skipping: set AI_BASE_URL + AI_API_KEY to run");
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
    let verdict = verify_grounding(&q, &chunk.text, &provider)
        .await
        .expect("verification failed");
    eprintln!(
        "real provider OK: dim={} | Q='{}' correct={:?} | grounded={} ({})",
        vecs[0].len(),
        q.text,
        q.correct_choices,
        verdict.supported,
        verdict.reason
    );
}
