//! The quiz source seam: where a live session's questions come from. The live
//! handler asks for a grounded question by query; the backing implementation is
//! either the RAG pipeline (retrieve → generate → verify) or a fixture for runs
//! without an AI provider or corpus.

use std::sync::Arc;

use async_trait::async_trait;

use presto_core::protocol::Question;
use presto_rag::corpus::Retriever;
use presto_rag::pipeline::grounded_question;
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
