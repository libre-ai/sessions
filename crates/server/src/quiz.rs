//! Content source seams: where a live session's questions and breakouts come
//! from. The live handler asks for a grounded question by query, or a grounded
//! clarification (breakout) by source-section id; the backing implementation is
//! either the RAG pipeline or a fixture for runs without an AI provider or corpus.

use std::sync::Arc;

use async_trait::async_trait;

use presto_core::protocol::{Flashcard, Question};
use presto_rag::corpus::Retriever;
use presto_rag::flashcards::flashcards;
use presto_rag::pipeline::{grounded_breakout, grounded_question};
use presto_rag::provider::AiProvider;

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
        grounded_question(query, self.retriever.as_ref(), self.provider.as_ref()).await
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
        grounded_breakout(section_id, self.retriever.as_ref(), self.provider.as_ref()).await
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
        flashcards(sections, self.retriever.as_ref(), self.provider.as_ref()).await
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
