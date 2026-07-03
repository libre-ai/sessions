//! Scoring hook module — extensible interface for custom answer evaluation.
//!
//! # Overview
//!
//! The `ScoreSink` trait allows products to implement custom scoring logic
//! for quiz/assessment sessions. The default tracer-bullet scoring formula
//! is: `correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0`.
//!
//! # Usage (for ai-practices)
//!
//! ```ignore
//! use presto_server::{InMemorySink, ScoreError, ScoreSink};
//! use async_trait::async_trait;
//!
//! struct DifficultyWeightedSink {
//!     inner: InMemorySink,
//!     difficulty_weights: HashMap<String, f64>,  // question_id -> weight
//! }
//!
//! #[async_trait]
//! impl ScoreSink for DifficultyWeightedSink {
//!     async fn compute_score(&self, question_id: &str, choice: &str, correct_choice: &str, elapsed_ms: u64)
//!         -> Result<u64, ScoreError>
//!     {
//!         let base = self.inner.compute_score(question_id, choice, correct_choice, elapsed_ms).await?;
//!         let weight = self.difficulty_weights.get(question_id).copied().unwrap_or(1.0);
//!         Ok((base as f64 * weight) as u64)
//!     }
//! }
//! ```

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::Mutex;

/// Error type for scoring hooks; suitable for async trait objects.
pub type ScoreError = Box<dyn std::error::Error + Send + Sync>;

/// Recorded answer entry: (session_id, participant_id, question_id, choice, elapsed_ms)
type RecordedAnswer = (String, String, String, String, u64);

/// Trait for custom scoring hook implementations.
/// Implement this trait to provide custom answer evaluation logic
/// (e.g., difficulty-weighted scoring, partial credit, etc.).
#[async_trait]
pub trait ScoreSink: Send + Sync {
    /// Called when a participant submits an answer.
    async fn on_answer_submitted(
        &self,
        session_id: &str,
        participant_id: &str,
        question_id: &str,
        choice: &str,
        elapsed_ms: u64,
    ) -> Result<(), ScoreError>;

    /// Compute score for an answer.
    /// Default formula: `correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0`
    async fn compute_score(
        &self,
        question_id: &str,
        choice: &str,
        correct_choice: &str,
        elapsed_ms: u64,
    ) -> Result<u64, ScoreError>;
}

/// In-memory mock ScoreSink for testing and local development.
pub struct InMemorySink {
    answers: Arc<Mutex<Vec<RecordedAnswer>>>,
}

impl InMemorySink {
    pub fn new() -> Self {
        Self {
            answers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn recorded_answers(&self) -> Vec<RecordedAnswer> {
        self.answers.lock().unwrap().clone()
    }
}

impl Default for InMemorySink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ScoreSink for InMemorySink {
    async fn on_answer_submitted(
        &self,
        session_id: &str,
        participant_id: &str,
        question_id: &str,
        choice: &str,
        elapsed_ms: u64,
    ) -> Result<(), ScoreError> {
        self.answers.lock().unwrap().push((
            session_id.to_string(),
            participant_id.to_string(),
            question_id.to_string(),
            choice.to_string(),
            elapsed_ms,
        ));
        Ok(())
    }

    async fn compute_score(
        &self,
        _question_id: &str,
        choice: &str,
        correct_choice: &str,
        elapsed_ms: u64,
    ) -> Result<u64, ScoreError> {
        if choice == correct_choice {
            let time_bonus =
                ((30000_i64 - elapsed_ms as i64).max(0) as f64 / 300.0).min(100.0) as u64;
            Ok(500 + time_bonus)
        } else {
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_score_hook_correct_answer() {
        let sink = InMemorySink::new();
        let score = sink.compute_score("q1", "A", "A", 5000).await.unwrap();
        assert_eq!(score, 583); // 500 + min((30000-5000)/300, 100) = 500 + 83
    }

    #[tokio::test]
    async fn test_score_hook_incorrect_answer() {
        let sink = InMemorySink::new();
        let score = sink.compute_score("q1", "B", "A", 5000).await.unwrap();
        assert_eq!(score, 0);
    }

    #[tokio::test]
    async fn test_score_hook_on_answer_submitted_recorded() {
        let sink = InMemorySink::new();
        sink.on_answer_submitted("sess1", "part1", "q1", "A", 5000)
            .await
            .unwrap();
        let recorded = sink.recorded_answers();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "sess1");
    }

    #[tokio::test]
    async fn test_score_hook_max_time_bonus() {
        // Maximum time bonus: elapsed_ms = 0 → bonus = 100
        let sink = InMemorySink::new();
        let score = sink.compute_score("q1", "A", "A", 0).await.unwrap();
        assert_eq!(score, 600); // 500 + 100
    }

    #[tokio::test]
    async fn test_score_hook_min_time_bonus() {
        // Beyond time window: elapsed_ms >= 30000 → bonus = 0
        let sink = InMemorySink::new();
        let score = sink.compute_score("q1", "A", "A", 30000).await.unwrap();
        assert_eq!(score, 500); // 500 + 0
    }
}
