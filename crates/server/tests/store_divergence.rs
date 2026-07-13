//! Divergence guard: the in-memory engine and the Postgres store must agree.
//! Both implement `SessionStore`; a subtle drift in the SQL re-implementation of
//! scoring/mastery (postgres_store.rs) vs the pure engine (session.rs) is a
//! trust-critical bug. This runs one scripted multi-round session against BOTH
//! and asserts identical leaderboard + mastery.
//!
//! The Postgres half requires `DATABASE_URL` (pgvector image works); ignored by
//! default. Run with:
//!
//! ```text
//! docker run --rm -d -p 5439:5432 -e POSTGRES_PASSWORD=presto --name presto-pgv pgvector/pgvector:pg16
//! DATABASE_URL=postgres://postgres:presto@127.0.0.1:5439/postgres \
//!   cargo test -p presto-server --test store_divergence -- --ignored --nocapture
//! ```

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex as AsyncMutex;

use presto_core::protocol::{LeaderboardEntry, Question, QuestionKind};
use presto_server::session::SectionMastery;
use presto_server::store::{InMemorySessionStore, SessionStore};

fn q(id: &str, section: &str, correct: u8) -> Question {
    Question {
        id: id.into(),
        text: "?".into(),
        kind: QuestionKind::Single,
        choices: vec!["a".into(), "b".into(), "c".into()],
        correct_choices: vec![correct],
        source_section_ids: vec![section.into()],
        citation_validation: None,
        timer_sec: 30,
    }
}

/// A fixed two-round script. Timestamps are pinned so both stores compute the
/// same elapsed-time speed bonus — any difference is a real divergence.
static DB_TEST_LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();

async fn db_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
    DB_TEST_LOCK
        .get_or_init(|| AsyncMutex::new(()))
        .lock()
        .await
}

async fn run(
    store: &dyn SessionStore,
    s: &str,
) -> (Vec<LeaderboardEntry>, Vec<Vec<SectionMastery>>) {
    store.ensure(s, "host").await.unwrap();
    store.join(s, "p1", "Alice").await.unwrap();
    store.join(s, "p2", "Bob").await.unwrap();
    store.join(s, "p3", "Carol").await.unwrap();

    // Round 1 — section A, correct = 0.
    store.push_question(s, &q("q1", "A", 0), 0).await.unwrap();
    store
        .submit_answer(s, "p1", "q1", vec![0], 1_000)
        .await
        .unwrap(); // correct, fast
    store
        .submit_answer(s, "p2", "q1", vec![1], 2_000)
        .await
        .unwrap(); // wrong
    store
        .submit_answer(s, "p3", "q1", vec![0], 5_000)
        .await
        .unwrap(); // correct, slower
    store.reveal(s).await.unwrap();

    // Round 2 — section B, correct = 1.
    store.push_question(s, &q("q2", "B", 1), 0).await.unwrap();
    store
        .submit_answer(s, "p1", "q2", vec![1], 3_000)
        .await
        .unwrap(); // correct
    store
        .submit_answer(s, "p2", "q2", vec![1], 1_000)
        .await
        .unwrap(); // correct, fast
    store
        .submit_answer(s, "p3", "q2", vec![2], 1_000)
        .await
        .unwrap(); // wrong
    let reveal = store.reveal(s).await.unwrap();

    let mut mastery = Vec::new();
    for p in ["p1", "p2", "p3"] {
        let mut m = store.mastery(s, p).await.unwrap();
        m.sort_by(|a, b| a.section_id.cmp(&b.section_id));
        mastery.push(m);
    }
    (reveal.leaderboard, mastery)
}

#[tokio::test]
#[ignore = "requires DATABASE_URL; see module docs"]
async fn inmemory_and_postgres_agree_over_a_multi_round_session() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };
    let _guard = db_test_lock().await;
    use presto_server::postgres_store::PostgresSessionStore;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let s_pg = format!("div-{nanos}");

    let mem = InMemorySessionStore::new();
    let pg = PostgresSessionStore::connect(&url).await.unwrap();

    let (mem_board, mem_mastery) = run(&mem, "div-mem").await;
    let (pg_board, pg_mastery) = run(&pg, &s_pg).await;

    assert_eq!(
        mem_board, pg_board,
        "leaderboard diverged between in-memory and Postgres"
    );
    assert_eq!(
        mem_mastery, pg_mastery,
        "mastery diverged between in-memory and Postgres"
    );
    // Sanity: the script is non-trivial (p1 leads, mastery accumulated 2 sections).
    assert_eq!(mem_board.len(), 3);
    assert_eq!(mem_mastery[0].len(), 2, "p1 answered two distinct sections");
    eprintln!("in-memory and Postgres agree: leaderboard {mem_board:?}");
}

async fn run_reusing_question_id(
    store: &dyn SessionStore,
    s: &str,
) -> (Vec<LeaderboardEntry>, Vec<Vec<SectionMastery>>) {
    store.ensure(s, "host").await.unwrap();
    store.join(s, "p1", "Alice").await.unwrap();
    store.join(s, "p2", "Bob").await.unwrap();

    // Round 1: q1, section A.
    store.push_question(s, &q("q1", "A", 1), 0).await.unwrap();
    store
        .submit_answer(s, "p1", "q1", vec![1], 30_000)
        .await
        .unwrap();
    store
        .submit_answer(s, "p2", "q1", vec![0], 30_000)
        .await
        .unwrap();
    let first = store.reveal(s).await.unwrap();
    assert_eq!(first.leaderboard[0].score, 500);
    assert_eq!(first.leaderboard[1].score, 0);

    // Round 2: the same deterministic q1 opens again; old answers must be gone.
    store.push_question(s, &q("q1", "A", 1), 0).await.unwrap();
    store
        .submit_answer(s, "p1", "q1", vec![0], 30_000)
        .await
        .unwrap();
    store
        .submit_answer(s, "p2", "q1", vec![1], 30_000)
        .await
        .unwrap();
    let reveal = store.reveal(s).await.unwrap();

    let mut mastery = Vec::new();
    for p in ["p1", "p2"] {
        let mut m = store.mastery(s, p).await.unwrap();
        m.sort_by(|a, b| a.section_id.cmp(&b.section_id));
        mastery.push(m);
    }
    (reveal.leaderboard, mastery)
}

#[tokio::test]
#[ignore = "requires DATABASE_URL; see module docs"]
async fn inmemory_and_postgres_agree_when_reusing_the_same_question_id() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };
    let _guard = db_test_lock().await;
    use presto_server::postgres_store::PostgresSessionStore;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let s_pg = format!("div-reuse-{nanos}");

    let mem = InMemorySessionStore::new();
    let pg = PostgresSessionStore::connect(&url).await.unwrap();

    let (mem_board, mem_mastery) = run_reusing_question_id(&mem, "div-reuse-mem").await;
    let (pg_board, pg_mastery) = run_reusing_question_id(&pg, &s_pg).await;

    assert_eq!(
        mem_board, pg_board,
        "leaderboard diverged on reused question_id"
    );
    assert_eq!(
        mem_mastery, pg_mastery,
        "mastery diverged on reused question_id"
    );
    assert_eq!(mem_board[0].score, 500);
    assert_eq!(mem_board[1].score, 500);
    assert_eq!(mem_mastery[0][0].correct, 1);
    assert_eq!(mem_mastery[0][0].total, 2);
    eprintln!("in-memory and Postgres agree on reused q1 rounds: {mem_board:?}");
}
