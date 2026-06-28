//! The grounded-quiz pipeline: **retrieve → generate → verify**. A candidate
//! question is generated from each retrieved chunk and kept only if the
//! grounding-verifier confirms it is supported by that chunk's source text.
//! Verification is a gate: unsupported (or unverifiable) questions are dropped,
//! never surfaced to a live session.

use presto_core::protocol::Question;

use crate::clarify::clarify;
use crate::corpus::{Chunk, Retriever};
use crate::generate::generate_from_chunk;
use crate::provider::AiProvider;
use crate::verify::verify_grounding;

/// Build up to `count` grounded questions for `query`. Retrieval or generation
/// failures and ungrounded candidates are skipped, so the result may be shorter
/// than `count` (possibly empty).
pub async fn grounded_quiz(
    query: &str,
    count: usize,
    retriever: &dyn Retriever,
    provider: &dyn AiProvider,
) -> Vec<Question> {
    let Ok(retrieved) = retriever.retrieve(query, count, provider).await else {
        return Vec::new();
    };

    let mut questions = Vec::new();
    for hit in retrieved {
        let chunk = Chunk {
            source_section_id: hit.source_section_id,
            text: hit.text,
        };
        let Ok(question) = generate_from_chunk(&chunk, provider).await else {
            continue;
        };
        // The verifier gates the question against its own source text.
        if let Ok(verdict) = verify_grounding(&question, &chunk.text, provider).await
            && verdict.supported
        {
            questions.push(question);
        }
    }
    questions
}

/// One grounded question for `query`, or `None` if nothing relevant was found or
/// the candidate failed grounding verification.
pub async fn grounded_question(
    query: &str,
    retriever: &dyn Retriever,
    provider: &dyn AiProvider,
) -> Option<Question> {
    grounded_quiz(query, 1, retriever, provider)
        .await
        .into_iter()
        .next()
}

/// A grounded clarification (breakout) for a confused source section, or `None`
/// if the section is not in the corpus or clarification fails.
pub async fn grounded_breakout(
    section_id: &str,
    retriever: &dyn Retriever,
    provider: &dyn AiProvider,
) -> Option<String> {
    let chunk = retriever.fetch_section(section_id).await.ok()??;
    clarify(&chunk, provider).await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    use crate::corpus::{CorpusError, Retrieved};
    use crate::provider::AiError;

    struct FakeRetriever {
        chunks: Vec<Retrieved>,
    }

    #[async_trait]
    impl Retriever for FakeRetriever {
        async fn retrieve(
            &self,
            _query: &str,
            k: usize,
            _provider: &dyn AiProvider,
        ) -> Result<Vec<Retrieved>, CorpusError> {
            Ok(self.chunks.iter().take(k).cloned().collect())
        }

        async fn fetch_section(&self, section_id: &str) -> Result<Option<Chunk>, CorpusError> {
            Ok(self
                .chunks
                .iter()
                .find(|c| c.source_section_id == section_id)
                .map(|c| Chunk {
                    source_section_id: c.source_section_id.clone(),
                    text: c.text.clone(),
                }))
        }
    }

    /// Returns generation JSON for the generate prompt and verdict JSON for the
    /// verify prompt, mimicking a real provider's two roles.
    struct PipelineFake {
        verifier_supports: bool,
    }

    #[async_trait]
    impl AiProvider for PipelineFake {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(vec![])
        }
        async fn complete(&self, system: &str, _user: &str) -> Result<String, AiError> {
            if system.contains("grounding checker") {
                Ok(format!(
                    "{{\"supported\": {}, \"reason\": \"r\"}}",
                    self.verifier_supports
                ))
            } else {
                Ok(
                    "{\"text\":\"Q?\",\"choices\":[\"a\",\"b\",\"c\",\"d\"],\"correct_choices\":[0]}"
                        .to_string(),
                )
            }
        }
    }

    fn retriever() -> FakeRetriever {
        FakeRetriever {
            chunks: vec![
                Retrieved {
                    source_section_id: "d#p0".into(),
                    text: "alpha".into(),
                    distance: 0.0,
                },
                Retrieved {
                    source_section_id: "d#p1".into(),
                    text: "beta".into(),
                    distance: 0.1,
                },
            ],
        }
    }

    #[tokio::test]
    async fn builds_a_grounded_quiz_when_verified() {
        let provider = PipelineFake {
            verifier_supports: true,
        };
        let quiz = grounded_quiz("topic", 2, &retriever(), &provider).await;
        assert_eq!(quiz.len(), 2);
        assert_eq!(quiz[0].source_section_ids, vec!["d#p0".to_string()]);
    }

    #[tokio::test]
    async fn drops_questions_that_fail_grounding() {
        let provider = PipelineFake {
            verifier_supports: false,
        };
        let quiz = grounded_quiz("topic", 2, &retriever(), &provider).await;
        assert!(quiz.is_empty(), "ungrounded questions must be dropped");
    }

    #[tokio::test]
    async fn grounded_question_returns_the_first_verified() {
        let provider = PipelineFake {
            verifier_supports: true,
        };
        let q = grounded_question("topic", &retriever(), &provider).await;
        assert_eq!(q.unwrap().source_section_ids, vec!["d#p0".to_string()]);
    }

    #[tokio::test]
    async fn breakout_clarifies_a_known_section() {
        let provider = PipelineFake {
            verifier_supports: true,
        };
        let out = grounded_breakout("d#p0", &retriever(), &provider).await;
        assert!(out.is_some_and(|s| !s.is_empty()));
    }

    #[tokio::test]
    async fn breakout_is_none_for_an_unknown_section() {
        let provider = PipelineFake {
            verifier_supports: true,
        };
        assert!(
            grounded_breakout("nope#p9", &retriever(), &provider)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn skips_questions_when_verifier_returns_malformed_json() {
        struct MalformedVerifierFake;
        #[async_trait]
        impl Retriever for MalformedVerifierFake {
            async fn retrieve(
                &self,
                _query: &str,
                _k: usize,
                _provider: &dyn AiProvider,
            ) -> Result<Vec<Retrieved>, CorpusError> {
                Ok(vec![Retrieved {
                    source_section_id: "d#p0".into(),
                    text: "test".into(),
                    distance: 0.0,
                }])
            }

            async fn fetch_section(&self, _section_id: &str) -> Result<Option<Chunk>, CorpusError> {
                Ok(None)
            }
        }

        struct MalformedVerifierProvider;
        #[async_trait]
        impl AiProvider for MalformedVerifierProvider {
            async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
                Ok(vec![])
            }
            async fn complete(&self, system: &str, _user: &str) -> Result<String, AiError> {
                if system.contains("grounding checker") {
                    // Return malformed JSON for verifier (missing "reason" field)
                    Ok("{\"supported\": true}".to_string())
                } else {
                    // Return valid generation JSON
                    Ok(
                        "{\"text\":\"Q?\",\"choices\":[\"a\",\"b\",\"c\",\"d\"],\"correct_choices\":[0]}"
                            .to_string(),
                    )
                }
            }
        }

        let quiz = grounded_quiz(
            "topic",
            1,
            &MalformedVerifierFake,
            &MalformedVerifierProvider,
        )
        .await;
        assert!(
            quiz.is_empty(),
            "questions should be skipped when verifier returns malformed JSON"
        );
    }
}
