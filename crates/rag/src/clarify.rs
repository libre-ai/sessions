//! Grounded clarification of a source section — the "breakout". When a section
//! confused participants, this produces a short tutor-style explanation grounded
//! only in that section's text.

use crate::corpus::Chunk;
use crate::generate::GenError;
use crate::provider::AiProvider;

const SYSTEM: &str = "You are a tutor. In 2-4 sentences of plain text, explain the key idea of the \
    source so a student who just answered a question about it incorrectly understands it. Ground \
    the explanation ONLY in the source; introduce no outside facts.";

/// Produce a grounded clarification of `chunk`. Returns an error on a provider
/// failure or an empty response.
pub async fn clarify(chunk: &Chunk, provider: &dyn AiProvider) -> Result<String, GenError> {
    let user = format!("Source:\n{}", chunk.text);
    let explanation = provider.complete(SYSTEM, &user).await?;
    let explanation = explanation.trim();
    if explanation.is_empty() {
        return Err(GenError("empty clarification".into()));
    }
    Ok(explanation.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    use crate::provider::AiError;

    struct TutorFake;

    #[async_trait]
    impl AiProvider for TutorFake {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(vec![])
        }
        async fn complete(&self, _system: &str, user: &str) -> Result<String, AiError> {
            Ok(format!(
                "  Here is why: {}  ",
                user.replace("Source:\n", "")
            ))
        }
    }

    #[tokio::test]
    async fn clarify_returns_trimmed_grounded_text() {
        let chunk = Chunk {
            source_section_id: "doc#p0".into(),
            text: "Rust enforces memory safety.".into(),
        };
        let out = clarify(&chunk, &TutorFake).await.unwrap();
        assert!(out.starts_with("Here is why:"));
        assert!(out.contains("Rust enforces memory safety."));
    }
}
