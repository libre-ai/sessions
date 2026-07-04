#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use presto_server::{ScoreError, ScoreSink};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Example AI-Practices custom sink (difficulty-weighted).
    struct AIPracticesCustomSink {
        difficulty_weights: HashMap<String, f64>,
        recorded: Arc<Mutex<Vec<(String, u64)>>>,
    }

    impl AIPracticesCustomSink {
        fn new() -> Self {
            let mut weights = HashMap::new();
            weights.insert("q1".to_string(), 1.5); // q1 is hard
            weights.insert("q2".to_string(), 1.0); // q2 is normal
            Self {
                difficulty_weights: weights,
                recorded: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl ScoreSink for AIPracticesCustomSink {
        async fn on_answer_submitted(
            &self,
            _session_id: &str,
            _participant_id: &str,
            _question_id: &str,
            _choice: &str,
            _elapsed_ms: u64,
        ) -> Result<(), ScoreError> {
            Ok(())
        }

        async fn compute_score(
            &self,
            question_id: &str,
            choice: &str,
            correct_choice: &str,
            elapsed_ms: u64,
        ) -> Result<u64, ScoreError> {
            let base = if choice == correct_choice {
                let time_bonus =
                    ((30000_i64 - elapsed_ms as i64).max(0) as f64 / 300.0).min(100.0) as u64;
                500 + time_bonus
            } else {
                0
            };
            let weight = self
                .difficulty_weights
                .get(question_id)
                .copied()
                .unwrap_or(1.0);
            let final_score = (base as f64 * weight) as u64;
            self.recorded
                .lock()
                .unwrap()
                .push((question_id.to_string(), final_score));
            Ok(final_score)
        }
    }

    #[tokio::test]
    async fn test_ai_practices_custom_sink_consumption() {
        let sink = AIPracticesCustomSink::new();

        // Correct answer, hard question (weight 1.5).
        let score = sink.compute_score("q1", "A", "A", 5000).await.unwrap();
        let expected = (583.0 * 1.5) as u64; // 583 * 1.5 = 874
        assert_eq!(score, expected);
    }

    #[tokio::test]
    async fn test_ai_practices_sink_integration_with_session() {
        let sink = AIPracticesCustomSink::new();

        sink.on_answer_submitted("sess1", "part1", "q1", "A", 5000)
            .await
            .unwrap();
        let score = sink.compute_score("q1", "A", "A", 5000).await.unwrap();

        let recorded = sink.recorded.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "q1");
        assert_eq!(recorded[0].1, score);
    }

    #[tokio::test]
    async fn test_ai_practices_difficulty_weight_multiplier() {
        // Verify that weights are applied correctly.
        let mut sink = AIPracticesCustomSink::new();
        sink.difficulty_weights.insert("q1".to_string(), 2.0); // Double weight

        let score = sink.compute_score("q1", "A", "A", 5000).await.unwrap();
        let base = 583_u64; // base tracer-bullet score
        let expected = (base as f64 * 2.0) as u64; // should be ~1166
        assert_eq!(score, expected);
    }

    #[tokio::test]
    async fn test_ai_practices_incorrect_answer_zero_score() {
        let sink = AIPracticesCustomSink::new();

        let score = sink.compute_score("q1", "B", "A", 5000).await.unwrap();
        assert_eq!(score, 0); // Wrong answer, weight doesn't matter
    }
}
