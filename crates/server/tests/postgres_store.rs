//! Postgres-specific coverage of the integrity/lifecycle behaviours added in P3:
//! server-side answer deadline and the open-question snapshot. Requires Postgres;
//! ignored by default. Run with:
//!
//! ```text
//! docker run --rm -d -p 5439:5432 -e POSTGRES_PASSWORD=presto --name presto-pgv pgvector/pgvector:pg16
//! DATABASE_URL=postgres://postgres:presto@127.0.0.1:5439/postgres \
//!   cargo test --test postgres_store -- --ignored --nocapture
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

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

#[tokio::test]
#[ignore = "requires DATABASE_URL; see module docs"]
async fn postgres_enforces_deadline_and_snapshots_open_question() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };
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

    // Open at t=0; the question becomes snapshot-able for late joiners.
    store.push_question(&s, &question(), 0).await.unwrap();
    let snap = store.snapshot(&s).await.unwrap();
    assert_eq!(snap.unwrap().id, "q1");

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

    // Reveal accumulated per-section mastery exactly once: p1 answered doc#p0 correctly (1/1).
    let mastery = store.mastery(&s, "p1").await.unwrap();
    let doc = mastery.iter().find(|m| m.section_id == "doc#p0").unwrap();
    assert_eq!((doc.correct, doc.total), (1, 1));
    eprintln!("postgres deadline + snapshot + mastery behave correctly ✅");
}
