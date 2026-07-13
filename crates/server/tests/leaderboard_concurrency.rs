//! §1 KPI: the leaderboard is deterministic under concurrent answer submission.
//! Concurrent submits must never corrupt scores (no lost updates), and the reveal
//! ordering must be stable (score desc, participant-id tie-break) — identical
//! across repeated concurrent runs.

use std::sync::Arc;

use presto_core::fixtures::sample_quiz;
use presto_server::store::{InMemorySessionStore, SessionStore};

const PARTICIPANTS: usize = 20;

/// Run one round: 20 participants submit concurrently (even ids correct, odd ids
/// wrong), then reveal. Returns the leaderboard as (participant_id, score) pairs.
async fn run_round() -> Vec<(String, u32)> {
    let store = Arc::new(InMemorySessionStore::new());
    let session = "s";
    store.ensure(session, "host").await.unwrap();

    let question = sample_quiz().into_iter().next().unwrap();
    let correct = question.correct_choices.clone();
    // A choice not in the correct set (the question has 4 choices).
    let wrong = vec![(0u8..4).find(|c| !correct.contains(c)).unwrap()];

    for i in 0..PARTICIPANTS {
        store
            .join(session, &format!("p{i:02}"), &format!("P{i}"))
            .await
            .unwrap();
    }
    store.push_question(session, &question, 0).await.unwrap();

    // Submit concurrently — the store must serialize these without lost updates.
    let mut handles = Vec::with_capacity(PARTICIPANTS);
    for i in 0..PARTICIPANTS {
        let store = store.clone();
        let pid = format!("p{i:02}");
        let choices = if i % 2 == 0 {
            correct.clone()
        } else {
            wrong.clone()
        };
        handles.push(tokio::spawn(async move {
            store
                .submit_answer(session, &pid, "q1", choices, 100)
                .await
                .unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    store
        .reveal(session)
        .await
        .unwrap()
        .leaderboard
        .into_iter()
        .map(|e| (e.participant_id, e.score))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leaderboard_deterministic_under_concurrent_submissions() {
    let first = run_round().await;

    // Every participant is accounted for (no lost updates).
    assert_eq!(first.len(), PARTICIPANTS);

    // Determinism: repeat the concurrent race; the leaderboard is byte-identical.
    for _ in 0..4 {
        assert_eq!(
            run_round().await,
            first,
            "leaderboard must be identical across concurrent runs"
        );
    }

    // Ordering invariant: scores non-increasing, ties broken by ascending id.
    for w in first.windows(2) {
        assert!(
            w[0].1 > w[1].1 || (w[0].1 == w[1].1 && w[0].0 < w[1].0),
            "leaderboard must be (score desc, id asc): {:?} before {:?}",
            w[0],
            w[1]
        );
    }

    // The correct answerers (even ids) all scored equally (same elapsed) and rank
    // above the zero-scoring wrong answerers (odd ids).
    let scored = first.iter().filter(|(_, s)| *s > 0).count();
    assert_eq!(
        scored,
        PARTICIPANTS / 2,
        "exactly the even ids should score"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reveal_is_idempotent_under_concurrent_calls() {
    let store = Arc::new(InMemorySessionStore::new());
    let session = "s-reveal";
    store.ensure(session, "host").await.unwrap();
    store.join(session, "p1", "Alice").await.unwrap();
    store.join(session, "p2", "Bob").await.unwrap();

    let question = sample_quiz().into_iter().next().unwrap();
    let correct = question.correct_choices.clone();
    let wrong = vec![(0u8..4).find(|c| !correct.contains(c)).unwrap()];
    store.push_question(session, &question, 0).await.unwrap();
    store
        .submit_answer(session, "p1", "q1", correct.clone(), 100)
        .await
        .unwrap();
    store
        .submit_answer(session, "p2", "q1", wrong.clone(), 100)
        .await
        .unwrap();

    let baseline = store.reveal(session).await.unwrap();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        handles.push(tokio::spawn(
            async move { store.reveal(session).await.unwrap() },
        ));
    }
    for handle in handles {
        assert_eq!(handle.await.unwrap(), baseline);
    }
    assert_eq!(store.reveal(session).await.unwrap(), baseline);
}
