//! The grounding-verifier: the harness's gate principle applied to generated
//! content. Given a question and the source text it cites, it asks the provider
//! whether the question and its marked correct answer are fully supported by that
//! source alone — the anti-hallucination check before a question reaches a live
//! session. This is the bridge between the harness (gates) and the product.

use serde::Deserialize;

use presto_core::protocol::Question;

use crate::extract_json;
use crate::provider::{AiError, AiProvider};

const SYSTEM: &str = "You are a strict grounding checker. Decide whether the question AND its \
    marked correct answer are fully supported by the source text ALONE. Reply with strict JSON \
    {\"supported\": boolean, \"reason\": string}. If anything is unstated or needs outside \
    knowledge, set supported to false.";

/// The verifier's decision for one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundingVerdict {
    pub supported: bool,
    pub reason: String,
}

/// A verification failure (provider error or unparseable verdict).
#[derive(Debug)]
pub struct VerifyError(pub String);

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "verification error: {}", self.0)
    }
}

impl std::error::Error for VerifyError {}

impl From<AiError> for VerifyError {
    fn from(e: AiError) -> Self {
        VerifyError(e.to_string())
    }
}

#[derive(Deserialize)]
struct RawVerdict {
    supported: bool,
    reason: String,
}

/// Verify that `question` (and its marked correct answer) is grounded in
/// `source_text`. A failed parse or provider error is returned as an error; a
/// well-formed "not supported" is a [`GroundingVerdict`] with `supported = false`.
pub async fn verify_grounding(
    question: &Question,
    source_text: &str,
    provider: &dyn AiProvider,
) -> Result<GroundingVerdict, VerifyError> {
    let correct = question
        .choices
        .get(usize::from(question.correct_choice))
        .map(String::as_str)
        .unwrap_or("");
    let user = format!(
        "Source:\n{source_text}\n\nQuestion: {question_text}\nMarked correct answer: {correct}",
        question_text = question.text,
    );
    let raw = provider.complete(SYSTEM, &user).await?;
    let parsed: RawVerdict = serde_json::from_str(extract_json(&raw))
        .map_err(|e| VerifyError(format!("invalid verdict JSON: {e}")))?;
    Ok(GroundingVerdict {
        supported: parsed.supported,
        reason: parsed.reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct VerdictFake {
        supported: bool,
    }

    #[async_trait]
    impl AiProvider for VerdictFake {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(vec![])
        }
        async fn complete(&self, _system: &str, _user: &str) -> Result<String, AiError> {
            Ok(format!(
                "{{\"supported\": {}, \"reason\": \"derived from the source\"}}",
                self.supported
            ))
        }
    }

    fn question() -> Question {
        Question {
            id: "q:doc#p0".into(),
            text: "What does Rust enforce?".into(),
            choices: vec!["GC".into(), "memory safety".into()],
            correct_choice: 1,
            source_section_ids: vec!["doc#p0".into()],
            timer_sec: 20,
        }
    }

    #[tokio::test]
    async fn accepts_a_grounded_question() {
        let v = verify_grounding(
            &question(),
            "Rust enforces memory safety.",
            &VerdictFake { supported: true },
        )
        .await
        .unwrap();
        assert!(v.supported);
    }

    #[tokio::test]
    async fn reports_an_ungrounded_question() {
        let v = verify_grounding(
            &question(),
            "Paris is the capital of France.",
            &VerdictFake { supported: false },
        )
        .await
        .unwrap();
        assert!(!v.supported);
    }

    #[tokio::test]
    async fn rejects_malformed_verdict_json() {
        struct MalformedFake;
        #[async_trait]
        impl AiProvider for MalformedFake {
            async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
                Ok(vec![])
            }
            async fn complete(&self, _system: &str, _user: &str) -> Result<String, AiError> {
                // Missing "reason" field — deserialization will fail
                Ok("{\"supported\": true}".to_string())
            }
        }
        let err = verify_grounding(&question(), "any text", &MalformedFake)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid verdict JSON"));
    }
}
