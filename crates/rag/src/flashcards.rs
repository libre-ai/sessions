//! Spaced-repetition flashcards generated from the source sections a participant
//! struggled with. Each card carries an initial SM-2 state for an external SRS
//! scheduler; the persistent cross-day review loop lives outside the live session.

use serde::Deserialize;

use presto_core::protocol::Flashcard;

use crate::corpus::{Chunk, Retriever};
use crate::extract_json;
use crate::generate::GenError;
use crate::provider::AiProvider;

const SYSTEM: &str = "You write one spaced-repetition flashcard grounded ONLY in the source. \
    Reply with strict JSON {\"front\": string (a question or prompt), \"back\": string (the \
    concise answer)}. No prose, no markdown.";

/// SM-2 initial ease factor.
const DEFAULT_EASE: f32 = 2.5;

#[derive(Deserialize)]
struct Card {
    front: String,
    back: String,
}

/// Generate one grounded flashcard from a section chunk.
pub async fn flashcard_from_chunk(
    chunk: &Chunk,
    provider: &dyn AiProvider,
) -> Result<Flashcard, GenError> {
    let user = format!("Source:\n{}", chunk.text);
    let raw = provider.complete_json(SYSTEM, &user).await?;
    let card: Card = serde_json::from_str(extract_json(&raw))
        .map_err(|e| GenError(format!("invalid flashcard JSON: {e}")))?;
    if card.front.trim().is_empty() || card.back.trim().is_empty() {
        return Err(GenError("flashcard front/back must be non-empty".into()));
    }
    Ok(Flashcard {
        section_id: chunk.source_section_id.clone(),
        front: card.front,
        back: card.back,
        ease_factor: DEFAULT_EASE,
        interval_days: 0,
    })
}

/// Build a deck for the given (weak) sections, skipping sections absent from the
/// corpus or whose generation fails.
pub async fn flashcards(
    sections: &[String],
    retriever: &dyn Retriever,
    provider: &dyn AiProvider,
) -> Vec<Flashcard> {
    let mut deck = Vec::new();
    for section in sections {
        if let Ok(Some(chunk)) = retriever.fetch_section(section).await
            && let Ok(card) = flashcard_from_chunk(&chunk, provider).await
        {
            deck.push(card);
        }
    }
    deck
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    use crate::corpus::{CorpusError, Retrieved};
    use crate::provider::AiError;

    struct CardFake;
    #[async_trait]
    impl AiProvider for CardFake {
        async fn embed(&self, _t: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(vec![])
        }
        async fn complete(&self, _s: &str, _u: &str) -> Result<String, AiError> {
            Ok(r#"{"front":"What is X?","back":"X is Y."}"#.to_string())
        }
    }

    struct OneSectionRetriever;
    #[async_trait]
    impl Retriever for OneSectionRetriever {
        async fn retrieve(
            &self,
            _q: &str,
            _k: usize,
            _p: &dyn AiProvider,
        ) -> Result<Vec<Retrieved>, CorpusError> {
            Ok(vec![])
        }
        async fn fetch_section(&self, section_id: &str) -> Result<Option<Chunk>, CorpusError> {
            Ok((section_id == "doc#p0").then(|| Chunk {
                source_section_id: "doc#p0".into(),
                text: "X is Y.".into(),
            }))
        }
    }

    #[tokio::test]
    async fn builds_a_deck_for_known_sections_only() {
        let deck = flashcards(
            &["doc#p0".into(), "missing#p9".into()],
            &OneSectionRetriever,
            &CardFake,
        )
        .await;
        assert_eq!(deck.len(), 1);
        assert_eq!(deck[0].section_id, "doc#p0");
        assert_eq!(deck[0].front, "What is X?");
        assert_eq!(deck[0].ease_factor, 2.5);
        assert_eq!(deck[0].interval_days, 0);
    }
}
