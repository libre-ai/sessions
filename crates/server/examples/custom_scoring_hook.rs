//! Example: Implementing a custom ScoreSink for difficulty-weighted scoring.
//! This is the pattern ai-practices should follow.

use async_trait::async_trait;
use presto_server::{ScoreError, ScoreSink};
use std::collections::HashMap;

/// Example: Difficulty-weighted scoring for ai-practices.
/// Multiplies the base tracer-bullet score by a per-question difficulty weight.
struct DifficultyWeightedSink {
    // In real usage, load from AI-practices config or database.
    difficulty_weights: HashMap<String, f64>,
}

impl DifficultyWeightedSink {
    fn new() -> Self {
        let mut weights = HashMap::new();
        weights.insert("q1".to_string(), 1.5); // Hard
        weights.insert("q2".to_string(), 1.0); // Normal
        weights.insert("q3".to_string(), 0.75); // Easy
        Self {
            difficulty_weights: weights,
        }
    }
}

#[async_trait]
impl ScoreSink for DifficultyWeightedSink {
    async fn on_answer_submitted(
        &self,
        _session_id: &str,
        _participant_id: &str,
        _question_id: &str,
        _choice: &str,
        _elapsed_ms: u64,
    ) -> Result<(), ScoreError> {
        // ai-practices: log to their analytics pipeline.
        Ok(())
    }

    async fn compute_score(
        &self,
        question_id: &str,
        choice: &str,
        correct_choice: &str,
        elapsed_ms: u64,
    ) -> Result<u64, ScoreError> {
        // Base score using tracer-bullet formula.
        let base = if choice == correct_choice {
            let time_bonus =
                ((30000_i64 - elapsed_ms as i64).max(0) as f64 / 300.0).min(100.0) as u64;
            500 + time_bonus
        } else {
            0
        };

        // Apply difficulty weight.
        let weight = self
            .difficulty_weights
            .get(question_id)
            .copied()
            .unwrap_or(1.0);
        Ok((base as f64 * weight) as u64)
    }
}

#[tokio::main]
async fn main() -> Result<(), ScoreError> {
    let sink = DifficultyWeightedSink::new();

    // Example: Compute score for a correct answer on a hard question.
    let weighted_score = sink.compute_score("q1", "A", "A", 5000).await?;
    let expected = (583.0 * 1.5) as u64; // 583 (base) * 1.5 (weight) = 874
    println!(
        "Weighted score: {} (expected ~{})",
        weighted_score, expected
    );

    Ok(())
}
