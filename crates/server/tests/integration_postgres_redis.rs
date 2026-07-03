#[cfg(test)]
mod tests {
    use presto_server::InMemorySink;
    use presto_server::scoring::ScoreSink;

    #[tokio::test]
    async fn test_score_hook_mock_integration() {
        let sink = InMemorySink::new();

        sink.on_answer_submitted("sess1", "part1", "q1", "A", 5000)
            .await
            .unwrap();
        let score = sink.compute_score("A", "A", 5000).await.unwrap();

        assert_eq!(score, 583);
        assert_eq!(sink.recorded_answers().len(), 1);
    }

    #[tokio::test]
    #[ignore] // Requires live Postgres; run with: cargo test -- --ignored
    async fn test_postgres_session_persist() {
        // TODO(I2): Implement PostgresSessionStore::connect() integration test.
        // Requires live DATABASE_URL env var pointing to Postgres 16+.
        // Will be implemented in I3 when Biscuit middleware requires session state.
    }

    #[tokio::test]
    #[ignore] // Requires live Redis; run with: cargo test -- --ignored
    async fn test_redis_fanout_publish_subscribe() {
        // TODO(I2): Implement RedisFanout integration test.
        // Requires live REDIS_URL env var pointing to Redis 7+.
        // Will verify channel isolation (subscribe to different channel should not receive).
    }
}
