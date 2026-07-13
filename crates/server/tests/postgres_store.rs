//! Postgres-specific coverage of the integrity/lifecycle behaviours added in P3:
//! server-side answer deadline and the open-question snapshot. Requires Postgres;
//! ignored by default. Run with:
//!
//! ```text
//! docker run --rm -d -p 5439:5432 -e POSTGRES_PASSWORD=presto --name presto-pgv pgvector/pgvector:pg16
//! DATABASE_URL=postgres://postgres:presto@127.0.0.1:5439/postgres \
//!   cargo test --test postgres_store -- --ignored --nocapture
//! ```

use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{Barrier, Mutex as AsyncMutex};

use presto_core::protocol::Question;
use presto_server::postgres_store::PostgresSessionStore;
use presto_server::session::SessionError;
use presto_server::store::{SessionStore, StoreError};

fn question() -> Question {
    Question {
        id: "q1".into(),
        text: "?".into(),
        kind: presto_core::protocol::QuestionKind::Single,
        choices: vec!["a".into(), "b".into()],
        correct_choices: vec![1],
        source_section_ids: vec!["doc#p0".into()],
        citation_validation: None,
        timer_sec: 30, // close at 30_000 + grace (1_500) ms
    }
}

static DB_TEST_LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();

async fn db_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
    DB_TEST_LOCK
        .get_or_init(|| AsyncMutex::new(()))
        .lock()
        .await
}

#[tokio::test]
#[ignore = "requires DATABASE_URL; see module docs"]
async fn postgres_enforces_deadline_and_snapshots_open_question() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };
    let _guard = db_test_lock().await;
    let store = PostgresSessionStore::connect(&url).await.unwrap();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let s = format!("pgt-{nanos}");

    store.ensure(&s, "host").await.unwrap();
    assert!(store.exists(&s).await.unwrap());
    assert!(!store.exists("no-such-session").await.unwrap());
    store.join(&s, "p1", "Alice").await.unwrap();
    store.join(&s, "p2", "Bob").await.unwrap();

    // Lobby: nothing open to snapshot.
    assert!(store.snapshot(&s).await.unwrap().is_none());
    assert!(store.guest_snapshot(&s, "p1").await.unwrap().is_some());

    // Open at t=0; the question becomes snapshot-able for late joiners.
    store.push_question(&s, &question(), 0).await.unwrap();
    let snap = store.snapshot(&s).await.unwrap();
    assert_eq!(snap.unwrap().id, "q1");
    let asking = store.guest_snapshot(&s, "p1").await.unwrap().unwrap();
    assert!(asking.question.is_some());
    assert!(asking.reveal.is_none());

    // Past the timer + grace: the server closes the question to answers.
    let closed = store.submit_answer(&s, "p2", "q1", vec![1], 31_501).await;
    assert!(matches!(
        closed,
        Err(StoreError::Session(SessionError::AnswerClosed))
    ));

    // Within the window: accepted (server-timed).
    store
        .submit_answer(&s, "p1", "q1", vec![1], 1_000)
        .await
        .unwrap();

    let first = store.reveal(&s).await.unwrap();
    assert_eq!(store.reveal(&s).await.unwrap(), first);
    assert!(store.snapshot(&s).await.unwrap().is_none());
    let revealed = store.guest_snapshot(&s, "p1").await.unwrap().unwrap();
    assert!(revealed.reveal.is_some());
    assert!(revealed.question.is_some());

    // Reveal accumulated per-section mastery exactly once: p1 answered doc#p0 correctly (1/1).
    let mastery = store.mastery(&s, "p1").await.unwrap();
    let doc = mastery.iter().find(|m| m.section_id == "doc#p0").unwrap();
    assert_eq!((doc.correct, doc.total), (1, 1));
    eprintln!("postgres deadline + snapshot + mastery behave correctly ✅");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL; see module docs"]
async fn postgres_reuses_question_ids_without_replaying_old_answers() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };
    let _guard = db_test_lock().await;
    let store = PostgresSessionStore::connect(&url).await.unwrap();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let s = format!("pgt-reuse-{nanos}");

    store.ensure(&s, "host").await.unwrap();
    store.join(&s, "p1", "Alice").await.unwrap();
    store.join(&s, "p2", "Bob").await.unwrap();

    store.push_question(&s, &question(), 0).await.unwrap();
    store
        .submit_answer(&s, "p1", "q1", vec![1], 30_000)
        .await
        .unwrap();
    store
        .submit_answer(&s, "p2", "q1", vec![0], 30_000)
        .await
        .unwrap();
    let first = store.reveal(&s).await.unwrap();
    assert_eq!(first.leaderboard[0].participant_id, "p1");
    assert_eq!(first.leaderboard[0].score, 500);
    assert_eq!(first.leaderboard[1].score, 0);

    // Re-open the same deterministic question id: the previous answers must not
    // survive, and the new round must start from a clean slate.
    store.push_question(&s, &question(), 0).await.unwrap();
    assert!(store.snapshot(&s).await.unwrap().is_some());
    store
        .submit_answer(&s, "p1", "q1", vec![0], 30_000)
        .await
        .unwrap();
    store
        .submit_answer(&s, "p2", "q1", vec![1], 30_000)
        .await
        .unwrap();
    let second = store.reveal(&s).await.unwrap();
    assert_eq!(second.leaderboard[0].score, 500);
    assert_eq!(second.leaderboard[1].score, 500);
    assert_eq!(store.reveal(&s).await.unwrap(), second);

    let p1_mastery = store.mastery(&s, "p1").await.unwrap();
    let doc = p1_mastery
        .iter()
        .find(|m| m.section_id == "doc#p0")
        .unwrap();
    assert_eq!((doc.correct, doc.total), (1, 2));
    eprintln!("postgres question reuse resets answers without replaying old score ✅");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL; see module docs"]
async fn postgres_reveal_is_single_score_under_concurrency() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };
    let _guard = db_test_lock().await;
    let store = Arc::new(PostgresSessionStore::connect(&url).await.unwrap());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let s = format!("pgt-reveal-{nanos}");

    store.ensure(&s, "host").await.unwrap();
    store.join(&s, "p1", "Alice").await.unwrap();
    store.join(&s, "p2", "Bob").await.unwrap();
    store.push_question(&s, &question(), 0).await.unwrap();
    store
        .submit_answer(&s, "p1", "q1", vec![1], 30_000)
        .await
        .unwrap();
    store
        .submit_answer(&s, "p2", "q1", vec![0], 30_000)
        .await
        .unwrap();

    let start = Arc::new(Barrier::new(9));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let start = start.clone();
        let session_id = s.clone();
        handles.push(tokio::spawn(async move {
            start.wait().await;
            store.reveal(&session_id).await.unwrap()
        }));
    }
    start.wait().await;

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    assert!(results.iter().all(|r| r == &results[0]));
    assert_eq!(results[0].leaderboard[0].score, 500);
    assert_eq!(results[0].leaderboard[1].score, 0);
    assert_eq!(store.reveal(&s).await.unwrap(), results[0]);
    eprintln!("postgres concurrent reveal stays single-score ✅");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL; see module docs"]
async fn postgres_reveal_returns_the_cached_result_after_a_late_join() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };
    let _guard = db_test_lock().await;
    let store = Arc::new(PostgresSessionStore::connect(&url).await.unwrap());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let s = format!("pgt-cached-{nanos}");

    store.ensure(&s, "host").await.unwrap();
    store.join(&s, "p1", "Alice").await.unwrap();
    store.join(&s, "p2", "Bob").await.unwrap();
    store.push_question(&s, &question(), 0).await.unwrap();
    store
        .submit_answer(&s, "p1", "q1", vec![1], 30_000)
        .await
        .unwrap();
    store
        .submit_answer(&s, "p2", "q1", vec![0], 30_000)
        .await
        .unwrap();

    let first = store.reveal(&s).await.unwrap();
    store.join(&s, "p3", "Cara").await.unwrap();

    let start = Arc::new(Barrier::new(9));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let start = start.clone();
        let session_id = s.clone();
        handles.push(tokio::spawn(async move {
            start.wait().await;
            store.reveal(&session_id).await.unwrap()
        }));
    }
    start.wait().await;

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    assert!(results.iter().all(|r| r == &first));
    assert!(
        !results[0]
            .leaderboard
            .iter()
            .any(|entry| entry.participant_id == "p3")
    );
    assert_eq!(store.reveal(&s).await.unwrap(), first);
    eprintln!("postgres cached reveal stays exact after late join ✅");
}
