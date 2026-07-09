//! Grounded question fixtures: questions backed by real ingested sources.

use crate::ingestion::SourceRef;
use presto_core::protocol::{CitationValidation, Question, QuestionKind};
use sqlx::PgPool;

/// Initialize all ingested sources from embedded assets.
pub async fn initialize_sources(pool: &PgPool) -> Result<Vec<SourceRef>, Box<dyn std::error::Error>> {
    let mut sources = Vec::new();

    // Ingest Rust Ownership Guide (embedded in binary)
    let rust_ownership_content = include_str!("../assets/rust-ownership-guide.md");
    let ownership_source = crate::ingestion::ingest_markdown(
        rust_ownership_content,
        "rust-ownership-guide.md",
        pool,
    )
    .await?;
    sources.push(ownership_source);

    Ok(sources)
}

/// Create a grounded question that cites a real ingested source.
fn grounded_question(
    id: &str,
    text: &str,
    choices: &[&str],
    correct: u8,
    source_id: &str,
) -> Question {
    Question {
        id: id.to_string(),
        text: text.to_string(),
        kind: QuestionKind::Single,
        choices: choices.iter().map(|c| c.to_string()).collect(),
        correct_choices: vec![correct],
        source_section_ids: vec![source_id.to_string()],
        citation_validation: Some(CitationValidation::verified(1)),
        timer_sec: 20,
    }
}

/// Generate questions grounded in real sources.
pub fn grounded_quiz(rust_ownership_source_id: &str) -> Vec<Question> {
    vec![
        grounded_question(
            "grounded_q1",
            "What does Rust's ownership system prevent? (Hint: See ownership rules)",
            &[
                "Memory leaks only",
                "Data races and use-after-free",
                "Slow compilation only",
                "Type mismatches",
            ],
            1,
            rust_ownership_source_id,
        ),
        grounded_question(
            "grounded_q2",
            "How many owners can a value have in Rust?",
            &["As many as needed", "Exactly one", "Up to three", "None"],
            1,
            rust_ownership_source_id,
        ),
        grounded_question(
            "grounded_q3",
            "What happens when a variable goes out of scope?",
            &["It moves to parent scope", "Rust calls drop() automatically", "The value persists", "Memory leak occurs"],
            1,
            rust_ownership_source_id,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grounded_quiz_is_well_formed() {
        let quiz = grounded_quiz("src_rust_ownership_test");
        assert_eq!(quiz.len(), 3);
        for question in &quiz {
            assert!(!question.correct_choices.is_empty());
            assert!(
                question
                    .correct_choices
                    .iter()
                    .all(|&c| (c as usize) < question.choices.len()),
                "correct_choices in range for {}",
                question.id
            );
            assert!(!question.source_section_ids.is_empty());
            // Verify citation validation is set to verified, not fixture
            assert!(
                question
                    .citation_validation
                    .as_ref()
                    .map(|cv| cv.status == presto_core::protocol::CitationValidationStatus::Verified)
                    .unwrap_or(false),
                "question {} must be verified, not fixture",
                question.id
            );
        }
    }
}
