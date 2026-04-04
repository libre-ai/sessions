//! A small hardcoded quiz for the live tracer-bullet (no RAG yet). Replaced by
//! grounded generation from an ingested corpus in P1/P2.

use crate::protocol::{Question, QuestionKind};

fn q(id: &str, text: &str, choices: &[&str], correct: u8, section: &str) -> Question {
    Question {
        id: id.to_string(),
        text: text.to_string(),
        kind: QuestionKind::Single,
        choices: choices.iter().map(|c| c.to_string()).collect(),
        correct_choices: vec![correct],
        source_section_ids: vec![section.to_string()],
        timer_sec: 20,
    }
}

/// Five sample questions, each tagged with a source section for the heatmap.
pub fn sample_quiz() -> Vec<Question> {
    vec![
        q(
            "q1",
            "Capital of France?",
            &["Paris", "Lyon", "Nice", "Brest"],
            0,
            "geo#fr",
        ),
        q("q2", "2 + 2 = ?", &["3", "4", "5", "22"], 1, "math#add"),
        q(
            "q3",
            "Rust's ownership prevents…",
            &["GC pauses", "data races", "slow builds", "type errors"],
            1,
            "rust#ownership",
        ),
        q(
            "q4",
            "HTTP status for Not Found?",
            &["200", "301", "404", "500"],
            2,
            "web#http",
        ),
        q(
            "q5",
            "Biscuit attenuation can only…",
            &[
                "extend rights",
                "restrict rights",
                "rotate keys",
                "revoke tokens",
            ],
            1,
            "auth#biscuit",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiz_is_well_formed() {
        let quiz = sample_quiz();
        assert_eq!(quiz.len(), 5);
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
        }
    }
}
