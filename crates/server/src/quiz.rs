//! Content source seams: where a live session's questions and breakouts come
//! from. The live handler asks for a grounded question by query, or a grounded
//! clarification (breakout) by source-section id; the backing implementation is
//! either the RAG pipeline or a fixture for runs without an AI provider or corpus.

use std::sync::Arc;

use async_trait::async_trait;

use presto_core::protocol::{Flashcard, Question};
use presto_rag::corpus::{CorpusStore, RetrievalScope, Retriever};
use presto_rag::flashcards::flashcards;
use presto_rag::ingest::document_text;
use presto_rag::pipeline::{grounded_breakout, grounded_question};
use presto_rag::provider::AiProvider;

// Single-tenant wedge: all content lives in the `default` space at the public
// level. When sessions carry a space + audience clearance (SP-A/SP-B), the scope
// becomes per-session — this is the single wiring point for that.
fn wedge_scope() -> RetrievalScope {
    RetrievalScope::wedge()
}

/// Produces grounded questions for a live session.
#[async_trait]
pub trait QuizSource: Send + Sync {
    /// One grounded question for `query`, or `None` when none can be produced
    /// (nothing retrieved, or the candidate failed grounding verification).
    async fn next_question(&self, query: &str) -> Option<Question>;
}

/// Backed by the RAG pipeline: retrieve → generate → verify.
pub struct RagQuizSource {
    retriever: Arc<dyn Retriever>,
    provider: Arc<dyn AiProvider>,
}

impl RagQuizSource {
    pub fn new(retriever: Arc<dyn Retriever>, provider: Arc<dyn AiProvider>) -> Self {
        Self {
            retriever,
            provider,
        }
    }
}

#[async_trait]
impl QuizSource for RagQuizSource {
    async fn next_question(&self, query: &str) -> Option<Question> {
        grounded_question(
            &wedge_scope(),
            query,
            self.retriever.as_ref(),
            self.provider.as_ref(),
        )
        .await
    }
}

/// Fixture-backed source for local runs without a corpus or AI provider. Returns
/// the sample quiz's first question regardless of `query`.
pub struct FixtureQuizSource;

#[async_trait]
impl QuizSource for FixtureQuizSource {
    async fn next_question(&self, _query: &str) -> Option<Question> {
        presto_core::fixtures::sample_quiz().into_iter().next()
    }
}

/// Grounded source: real questions backed by ingested sources (gear-loader).
/// Rotates through grounded questions; source_id embedded in first question.
pub struct GroundedQuizSource {
    questions: Vec<Question>,
    index: Arc<parking_lot::Mutex<usize>>,
}

impl GroundedQuizSource {
    pub fn new(source_id: &str) -> Self {
        let questions = crate::grounded_fixtures::grounded_quiz(source_id);
        Self {
            questions,
            index: Arc::new(parking_lot::Mutex::new(0)),
        }
    }
}

#[async_trait]
impl QuizSource for GroundedQuizSource {
    async fn next_question(&self, _query: &str) -> Option<Question> {
        if self.questions.is_empty() {
            return None;
        }
        let mut idx = self.index.lock();
        let question = self.questions[*idx % self.questions.len()].clone();
        *idx = (*idx + 1) % self.questions.len();
        Some(question)
    }
}

/// Produces grounded clarifications (breakouts) for a confused source section.
#[async_trait]
pub trait BreakoutSource: Send + Sync {
    /// A grounded clarification for `section_id`, or `None` if none can be made.
    async fn breakout(&self, section_id: &str) -> Option<String>;
}

/// Backed by the RAG pipeline: fetch the section → grounded clarification.
pub struct RagBreakoutSource {
    retriever: Arc<dyn Retriever>,
    provider: Arc<dyn AiProvider>,
}

impl RagBreakoutSource {
    pub fn new(retriever: Arc<dyn Retriever>, provider: Arc<dyn AiProvider>) -> Self {
        Self {
            retriever,
            provider,
        }
    }
}

#[async_trait]
impl BreakoutSource for RagBreakoutSource {
    async fn breakout(&self, section_id: &str) -> Option<String> {
        grounded_breakout(
            &wedge_scope(),
            section_id,
            self.retriever.as_ref(),
            self.provider.as_ref(),
        )
        .await
    }
}

/// Fixture breakout for runs without a corpus or AI provider.
pub struct FixtureBreakoutSource;

#[async_trait]
impl BreakoutSource for FixtureBreakoutSource {
    async fn breakout(&self, section_id: &str) -> Option<String> {
        Some(format!(
            "(demo) Review the source for section {section_id} — connect a corpus + AI provider \
             for a grounded clarification."
        ))
    }
}

/// Produces a spaced-repetition flashcard deck for a set of weak sections.
#[async_trait]
pub trait FlashcardSource: Send + Sync {
    async fn deck(&self, sections: &[String]) -> Vec<Flashcard>;
}

/// Backed by the RAG pipeline: fetch each section → grounded flashcard.
pub struct RagFlashcardSource {
    retriever: Arc<dyn Retriever>,
    provider: Arc<dyn AiProvider>,
}

impl RagFlashcardSource {
    pub fn new(retriever: Arc<dyn Retriever>, provider: Arc<dyn AiProvider>) -> Self {
        Self {
            retriever,
            provider,
        }
    }
}

#[async_trait]
impl FlashcardSource for RagFlashcardSource {
    async fn deck(&self, sections: &[String]) -> Vec<Flashcard> {
        flashcards(
            &wedge_scope(),
            sections,
            self.retriever.as_ref(),
            self.provider.as_ref(),
        )
        .await
    }
}

/// Why an ingestion was refused. Separates client-safe document faults from
/// backend failures whose detail must not leak to an untrusted uploader.
pub enum IngestRejection {
    /// The uploaded document is unusable (bad type, encoding, empty). The
    /// message is safe to return to the client.
    BadDocument(String),
    /// No corpus + AI provider is configured on this deployment.
    NotConfigured,
    /// A backend failure (database, provider). Detail is logged, never returned.
    Backend,
}

/// Ingests an uploaded document into the corpus (parse → chunk → embed → store),
/// so the RAG question/breakout/flashcard sources can ground on it.
#[async_trait]
pub trait DocumentIngestor: Send + Sync {
    /// Ingest `bytes` (typed by `content_type`) under `document_id`; returns the
    /// number of chunks stored, or a typed rejection.
    async fn ingest(
        &self,
        document_id: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<usize, IngestRejection>;
}

/// Parses the body to text, then ingests it into the pgvector corpus.
pub struct RagIngestor {
    corpus: Arc<CorpusStore>,
    provider: Arc<dyn AiProvider>,
}

impl RagIngestor {
    pub fn new(corpus: Arc<CorpusStore>, provider: Arc<dyn AiProvider>) -> Self {
        Self { corpus, provider }
    }
}

#[async_trait]
impl DocumentIngestor for RagIngestor {
    async fn ingest(
        &self,
        document_id: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<usize, IngestRejection> {
        let text = document_text(content_type, bytes)
            .map_err(|e| IngestRejection::BadDocument(e.to_string()))?;
        self.corpus
            // Wedge: ingest into the `default` space at the public level.
            .ingest("default", 0, document_id, &text, self.provider.as_ref())
            .await
            .map_err(|e| {
                // Log the detail; return an opaque rejection (no DB internals to
                // an untrusted uploader).
                eprintln!("ingest backend error for '{document_id}': {e}");
                IngestRejection::Backend
            })
    }
}

/// Rejects ingestion when no corpus + AI provider is configured.
pub struct FixtureIngestor;

#[async_trait]
impl DocumentIngestor for FixtureIngestor {
    async fn ingest(&self, _doc: &str, _ct: &str, _bytes: &[u8]) -> Result<usize, IngestRejection> {
        Err(IngestRejection::NotConfigured)
    }
}

/// Fixture flashcards for runs without a corpus or AI provider.
pub struct FixtureFlashcardSource;

#[async_trait]
impl FlashcardSource for FixtureFlashcardSource {
    async fn deck(&self, sections: &[String]) -> Vec<Flashcard> {
        sections
            .iter()
            .map(|s| Flashcard {
                section_id: s.clone(),
                front: format!("Review section {s}"),
                back: "(demo) connect a corpus + AI provider for a grounded card.".into(),
                ease_factor: 2.5,
                interval_days: 0,
            })
            .collect()
    }
}
