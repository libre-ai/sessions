//! presto-rag — ingestion, retrieval, and grounded generation for Presto-Matic.
//!
//! - [`provider`] — the AI provider seam (OpenAI-compatible client + fake).
//! - [`corpus`] — ingestion (chunk → embed) and retrieval over Postgres + pgvector.
//! - [`generate`] — grounded quiz-question generation from corpus chunks.
//! - [`verify`] — the grounding-verifier: checks a generated question against its
//!   source before use (the harness's gate principle applied to content).
//!
//! Every module depends on the [`provider`] seam, keeping the product decoupled
//! from any single AI vendor.

pub mod clarify;
pub mod corpus;
pub mod flashcards;
pub mod generate;
pub mod ingest;
pub mod pipeline;
pub mod provider;
pub mod verify;

/// Extract the first top-level JSON object from `s`, tolerating markdown fences
/// or surrounding prose that a model may add. Returns `s` unchanged when no
/// braces are present.
pub(crate) fn extract_json(s: &str) -> &str {
    match (s.find('{'), s.rfind('}')) {
        (Some(start), Some(end)) if end >= start => &s[start..=end],
        _ => s,
    }
}
