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
}
