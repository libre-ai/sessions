//! Real notebook fixture orchestration.
//!
//! This module executes scoped retrieval, generation from a fenced untrusted
//! chunk, and the fail-closed lexical verifier. Its output is only a candidate:
//! it has no API capable of creating or selecting an approved permit.

use std::sync::Arc;

use async_trait::async_trait;
use presto_core::api::{ConfidentialityLevel, SourceCitation};
use presto_rag::corpus::{Chunk, CorpusError, RetrievalScope, Retrieved, Retriever};
use presto_rag::generate::generate_from_chunk;
use presto_rag::provider::{AiError, AiProvider};
use presto_rag::verify::verify_grounding;
use sha2::{Digest, Sha256};

const FIXTURE_SOURCE: &str = "La France a pour capitale Paris. Paris est la capitale de la France.";
const FIXTURE_TITLE: &str = "Référence géographique approuvée";
const MAX_RETRIEVED_CHUNKS: usize = 1;
const MAX_SOURCE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookRagError {
    Retrieval,
    Generation,
    Verification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookCandidate {
    pub(crate) answer: String,
    pub(crate) citation: SourceCitation,
    pub(crate) source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotebookRagOutcome {
    Candidate(NotebookCandidate),
    Rejected,
}

#[async_trait]
pub trait NotebookRagEngine: Send + Sync {
    async fn run(
        &self,
        space_id: &str,
        effective_clearance: ConfidentialityLevel,
        query: &str,
    ) -> Result<NotebookRagOutcome, NotebookRagError>;
}

pub struct StagedNotebookRagEngine {
    retriever: Arc<dyn Retriever>,
    provider: Arc<dyn AiProvider>,
}

impl StagedNotebookRagEngine {
    pub fn new(retriever: Arc<dyn Retriever>, provider: Arc<dyn AiProvider>) -> Self {
        Self {
            retriever,
            provider,
        }
    }

    pub fn fixture() -> Self {
        Self::new(
            Arc::new(ProvisionedFixtureRetriever),
            Arc::new(DeterministicNotebookProvider),
        )
    }
}

#[async_trait]
impl NotebookRagEngine for StagedNotebookRagEngine {
    async fn run(
        &self,
        space_id: &str,
        effective_clearance: ConfidentialityLevel,
        query: &str,
    ) -> Result<NotebookRagOutcome, NotebookRagError> {
        let scope = RetrievalScope {
            space_id: space_id.to_owned(),
            max_confidentiality: confidentiality_rank(effective_clearance),
        };
        let retrieved = self
            .retriever
            .retrieve(&scope, query, MAX_RETRIEVED_CHUNKS, self.provider.as_ref())
            .await
            .map_err(|_| NotebookRagError::Retrieval)?;
        let Some(retrieved) = retrieved.into_iter().next() else {
            return Ok(NotebookRagOutcome::Rejected);
        };
        if retrieved.text.is_empty() || retrieved.text.len() > MAX_SOURCE_BYTES {
            return Ok(NotebookRagOutcome::Rejected);
        }
        let chunk = Chunk {
            source_section_id: retrieved.source_section_id,
            text: retrieved.text,
        };
        let question = generate_from_chunk(&chunk, self.provider.as_ref())
            .await
            .map_err(|_| NotebookRagError::Generation)?;
        let verdict = verify_grounding(&question, &chunk, self.provider.as_ref())
            .await
            .map_err(|_| NotebookRagError::Verification)?;
        if !verdict.is_supported() || question.correct_choices.len() != 1 {
            return Ok(NotebookRagOutcome::Rejected);
        }
        let answer = question
            .correct_choices
            .first()
            .and_then(|index| question.choices.get(usize::from(*index)))
            .cloned()
            .ok_or(NotebookRagError::Generation)?;
        let document_id = chunk
            .source_section_id
            .split_once('#')
            .map(|(document_id, _)| document_id.to_owned());
        Ok(NotebookRagOutcome::Candidate(NotebookCandidate {
            answer,
            citation: SourceCitation {
                source_section_id: chunk.source_section_id.clone(),
                document_id,
                title: Some(FIXTURE_TITLE.to_owned()),
                excerpt: Some(chunk.text.clone()),
            },
            source_hash: scoped_source_hash(space_id, &chunk.source_section_id, &chunk.text),
        }))
    }
}

struct ProvisionedFixtureRetriever;

impl ProvisionedFixtureRetriever {
    /// Derives the scoped fixture on demand without retaining per-space state.
    fn fixture_for_scope(space_id: &str) -> Option<Chunk> {
        (!space_id.is_empty()).then(|| fixture_chunk(space_id))
    }
}

#[async_trait]
impl Retriever for ProvisionedFixtureRetriever {
    async fn retrieve(
        &self,
        scope: &RetrievalScope,
        _query: &str,
        k: usize,
        _provider: &dyn AiProvider,
    ) -> Result<Vec<Retrieved>, CorpusError> {
        if scope.max_confidentiality < confidentiality_rank(ConfidentialityLevel::Public) || k == 0
        {
            return Ok(Vec::new());
        }
        Ok(Self::fixture_for_scope(&scope.space_id)
            .into_iter()
            .map(|chunk| Retrieved {
                source_section_id: chunk.source_section_id,
                text: chunk.text,
                distance: 0.0,
            })
            .take(k)
            .collect())
    }

    async fn fetch_section(
        &self,
        scope: &RetrievalScope,
        section_id: &str,
    ) -> Result<Option<Chunk>, CorpusError> {
        Ok(Self::fixture_for_scope(&scope.space_id)
            .filter(|chunk| chunk.source_section_id == section_id))
    }
}

struct DeterministicNotebookProvider;

#[async_trait]
impl AiProvider for DeterministicNotebookProvider {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        Ok(vec![vec![1.0]])
    }

    async fn complete(&self, system: &str, user: &str) -> Result<String, AiError> {
        self.complete_json(system, user).await
    }

    async fn complete_json(&self, system: &str, user: &str) -> Result<String, AiError> {
        if system.contains("strict grounding checker") {
            let source_section_id = user
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("Expected source section id: "))
                .ok_or_else(|| AiError("invalid fixture verifier prompt".into()))?;
            Ok(format!(
                "{{\"supported\":true,\"reason\":\"exact\",\"evidence\":{{\"source_section_id\":{source_section_id:?},\"exact_quote\":\"Paris est la capitale de la France.\"}}}}"
            ))
        } else {
            Ok("{\"text\":\"Quelle est la capitale de la France ?\",\"choices\":[\"Paris est la capitale de la France.\",\"Lyon\",\"Marseille\"],\"correct_choices\":[0]}".to_owned())
        }
    }
}

pub(crate) fn fixture_source_section_id(space_id: &str) -> String {
    format!("fixture-{}#france", short_scope_hash(space_id))
}

pub(crate) fn fixture_document_id(space_id: &str) -> String {
    format!("fixture-{}", short_scope_hash(space_id))
}

pub(crate) fn fixture_source_text() -> &'static str {
    FIXTURE_SOURCE
}

pub(crate) fn fixture_title() -> &'static str {
    FIXTURE_TITLE
}

pub(crate) fn scoped_source_hash(space_id: &str, source_section_id: &str, text: &str) -> String {
    hash_fields(&[space_id, source_section_id, text])
}

fn fixture_chunk(space_id: &str) -> Chunk {
    Chunk {
        source_section_id: fixture_source_section_id(space_id),
        text: FIXTURE_SOURCE.to_owned(),
    }
}

fn short_scope_hash(space_id: &str) -> String {
    hash_fields(&["notebook-fixture-space-v1", space_id])
        .chars()
        .take(16)
        .collect()
}

fn hash_fields(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const fn confidentiality_rank(level: ConfidentialityLevel) -> i16 {
    match level {
        ConfidentialityLevel::Public => 0,
        ConfidentialityLevel::Internal => 1,
        ConfidentialityLevel::Confidential => 2,
        ConfidentialityLevel::Secret => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestRetriever {
        result: Result<Option<Chunk>, ()>,
    }

    #[async_trait]
    impl Retriever for TestRetriever {
        async fn retrieve(
            &self,
            _scope: &RetrievalScope,
            _query: &str,
            _k: usize,
            _provider: &dyn AiProvider,
        ) -> Result<Vec<Retrieved>, CorpusError> {
            match &self.result {
                Err(()) => Err(CorpusError("secret retrieval detail".into())),
                Ok(None) => Ok(Vec::new()),
                Ok(Some(chunk)) => Ok(vec![Retrieved {
                    source_section_id: chunk.source_section_id.clone(),
                    text: chunk.text.clone(),
                    distance: 0.0,
                }]),
            }
        }

        async fn fetch_section(
            &self,
            _scope: &RetrievalScope,
            _section_id: &str,
        ) -> Result<Option<Chunk>, CorpusError> {
            Ok(None)
        }
    }

    #[derive(Clone, Copy)]
    enum ProviderMode {
        Valid,
        Error,
        MalformedGeneration,
        MalformedVerifier,
    }

    struct TestProvider {
        mode: ProviderMode,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl AiProvider for TestProvider {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(vec![])
        }

        async fn complete(&self, system: &str, user: &str) -> Result<String, AiError> {
            self.complete_json(system, user).await
        }

        async fn complete_json(&self, system: &str, user: &str) -> Result<String, AiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                ProviderMode::Error => Err(AiError("secret provider detail".into())),
                ProviderMode::MalformedGeneration if !system.contains("strict grounding checker") => {
                    Ok("not json".into())
                }
                ProviderMode::MalformedVerifier if system.contains("strict grounding checker") => {
                    Ok("not json".into())
                }
                _ if system.contains("strict grounding checker") => {
                    let source_section_id = user
                        .lines()
                        .next()
                        .unwrap()
                        .strip_prefix("Expected source section id: ")
                        .unwrap();
                    Ok(format!("{{\"supported\":true,\"reason\":\"exact\",\"evidence\":{{\"source_section_id\":{source_section_id:?},\"exact_quote\":\"Paris est la capitale de la France.\"}}}}"))
                }
                _ => Ok("{\"text\":\"Capital?\",\"choices\":[\"Paris est la capitale de la France.\",\"Lyon\"],\"correct_choices\":[0]}".into()),
            }
        }
    }

    fn engine(result: Result<Option<Chunk>, ()>, mode: ProviderMode) -> StagedNotebookRagEngine {
        StagedNotebookRagEngine::new(
            Arc::new(TestRetriever { result }),
            Arc::new(TestProvider {
                mode,
                calls: AtomicUsize::new(0),
            }),
        )
    }

    fn source() -> Chunk {
        Chunk {
            source_section_id: "fixture-test#p0".into(),
            text: FIXTURE_SOURCE.into(),
        }
    }

    #[tokio::test]
    async fn fixture_executes_all_stages_and_returns_a_candidate() {
        let outcome = StagedNotebookRagEngine::fixture()
            .run("space-a", ConfidentialityLevel::Public, "capital?")
            .await
            .unwrap();
        let NotebookRagOutcome::Candidate(candidate) = outcome else {
            panic!("fixture must produce a candidate")
        };
        assert_eq!(candidate.answer, "Paris est la capitale de la France.");
        assert_eq!(
            candidate.citation.source_section_id,
            fixture_source_section_id("space-a")
        );
    }

    #[tokio::test]
    async fn absent_source_is_rejected_without_generation() {
        assert_eq!(
            engine(Ok(None), ProviderMode::Valid)
                .run("space-a", ConfidentialityLevel::Public, "capital?")
                .await,
            Ok(NotebookRagOutcome::Rejected)
        );
    }

    #[tokio::test]
    async fn retrieval_provider_malformed_generation_and_verifier_errors_are_bounded() {
        let cases = [
            (
                engine(Err(()), ProviderMode::Valid),
                NotebookRagError::Retrieval,
            ),
            (
                engine(Ok(Some(source())), ProviderMode::Error),
                NotebookRagError::Generation,
            ),
            (
                engine(Ok(Some(source())), ProviderMode::MalformedGeneration),
                NotebookRagError::Generation,
            ),
            (
                engine(Ok(Some(source())), ProviderMode::MalformedVerifier),
                NotebookRagError::Verification,
            ),
        ];
        for (engine, expected) in cases {
            assert_eq!(
                engine
                    .run("space-a", ConfidentialityLevel::Public, "capital?")
                    .await,
                Err(expected)
            );
        }
    }

    #[tokio::test]
    async fn fixture_is_derived_deterministically_without_cross_space_artifacts() {
        let engine = StagedNotebookRagEngine::fixture();
        let a = engine
            .run("space-a", ConfidentialityLevel::Public, "capital?")
            .await
            .unwrap();
        let a_again = engine
            .run("space-a", ConfidentialityLevel::Public, "capital?")
            .await
            .unwrap();
        let b = engine
            .run("space-b", ConfidentialityLevel::Public, "capital?")
            .await
            .unwrap();
        let (
            NotebookRagOutcome::Candidate(a),
            NotebookRagOutcome::Candidate(a_again),
            NotebookRagOutcome::Candidate(b),
        ) = (a, a_again, b)
        else {
            panic!("all non-empty scopes must produce candidates")
        };
        assert_eq!(a, a_again);
        assert_ne!(a.citation.source_section_id, b.citation.source_section_id);
        assert_ne!(a.source_hash, b.source_hash);
    }
}
