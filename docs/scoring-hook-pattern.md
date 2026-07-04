# Scoring Hook Pattern — Integration Guide for AI-Practices & Crew

## Overview

The `ScoreSink` trait lets consumers prototype custom scoring logic without modifying LM core scoring. This increment publishes the trait, an in-memory test sink, examples, and consumption tests. It does **not** wire custom scoring into the live session runtime yet.

## Step 1: Add LM dependency

```toml
# In your product's Cargo.toml during local integration
presto-server = { path = "$DEV_ROOT/rumble-lm/crates/server" }
```

## Step 2: Implement `ScoreSink`

```rust
use async_trait::async_trait;
use presto_server::{ScoreError, ScoreSink};

pub struct YourCustomSink {
    // Your state: question metadata, difficulty weights, analytics client, ...
}

#[async_trait]
impl ScoreSink for YourCustomSink {
    async fn on_answer_submitted(
        &self,
        session_id: &str,
        participant_id: &str,
        question_id: &str,
        choice: &str,
        elapsed_ms: u64,
    ) -> Result<(), ScoreError> {
        // Log, validate, trigger side-effects.
        Ok(())
    }

    async fn compute_score(
        &self,
        question_id: &str,
        choice: &str,
        correct_choice: &str,
        elapsed_ms: u64,
    ) -> Result<u64, ScoreError> {
        // Your scoring formula here.
        // Base tracer-bullet: correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0
        Ok(0)
    }
}
```

## Runtime integration status

`ScoreSink` is a published seam, not an active runtime dependency. The current LM live session engine still uses its built-in scoring path. A future runtime increment may inject a `ScoreSink` into session handling once a real consumer needs it; that work must include end-to-end tests proving the built-in default remains unchanged.

## Example: difficulty-weighted scoring (ai-practices use case)

See `$DEV_ROOT/rumble-lm/crates/server/examples/custom_scoring_hook.rs` for a working example.

Key points:

- Load difficulty weights from config or database.
- Use `question_id` to choose the weight.
- Multiply base tracer-bullet score by the per-question weight.
- Keep analytics side effects in `on_answer_submitted`.
- Test custom logic in isolation before any live-runtime integration.

## Testing your implementation

Use the `InMemorySink` mock from LM for unit tests:

```rust
use presto_server::{InMemorySink, ScoreSink};

#[tokio::test]
async fn test_scoring() {
    let sink = InMemorySink::new();
    let score = sink.compute_score("q1", "A", "A", 5000).await.unwrap();
    assert_eq!(score, 583);
}
```

## Tracer-bullet formula reference

Base scoring formula used by `InMemorySink`:

- Correct answer: `500 + min((30000 - elapsed_ms).max(0) / 300, 100)`
  - Speed bonus: max 100 points, min 0 for ≥30s.
- Incorrect answer: `0`.

Your custom sink can override this formula entirely.
