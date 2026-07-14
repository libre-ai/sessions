//! Correct multi-instance scoring: with the Postgres store, a participant who
//! answers on instance B is scored by a `reveal` run on instance A. This is the
//! property the in-memory store CANNOT provide. Requires Postgres + Redis;
//! ignored by default. Run with:
//!
//! ```text
//! docker run --rm -d -p 6399:6379 --name presto-redis redis:7-alpine
//! docker run --rm -d -p 5439:5432 -e POSTGRES_PASSWORD=presto --name presto-pg postgres:16-alpine
//! DATABASE_URL=postgres://postgres:presto@127.0.0.1:5439/postgres \
//! REDIS_URL=redis://127.0.0.1:6399/ \
//!   cargo test --test postgres_multi_instance -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_server::auth::{Auth, Capability};
use presto_server::postgres_store::PostgresSessionStore;
use presto_server::redis_fanout::RedisFanout;
use presto_server::{AppState, app};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn instance(db: &str, redis: &str, auth: Arc<Auth>) -> std::net::SocketAddr {
    let store = Arc::new(PostgresSessionStore::connect(db).await.expect("postgres"));
    let fanout = Arc::new(RedisFanout::connect(redis).await.expect("redis"));
    let state = AppState {
        store,
        fanout,
        owner_auth: Arc::new(presto_server::owner_auth::OwnerAuth::disabled(auth.clone())),
        owner_corpus: Arc::new(presto_server::owner_corpus::OwnerCorpusStore::new()),
        approved_claims: Arc::new(presto_server::approved_claims::ApprovedClaimRegistry::fixture()),
        notebook_rag: Arc::new(presto_server::notebook_rag::StagedNotebookRagEngine::fixture()),
        auth,
        quiz: Arc::new(presto_server::quiz::FixtureQuizSource),
        breakout: Arc::new(presto_server::quiz::FixtureBreakoutSource),
        flashcards: Arc::new(presto_server::quiz::FixtureFlashcardSource),
        ingestor: Arc::new(presto_server::quiz::FixtureIngestor),
        legacy_ingest_token: None,
        session_rate: Arc::new(presto_server::ratelimit::TokenBucket::new(1000.0, 1000.0)),
        join_redemption_rate: Arc::new(presto_server::ratelimit::TokenBucket::new(1000.0, 1000.0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    addr
}

async fn recv_until(ws: &mut Ws, kind: &str) -> Value {
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["type"] == kind {
                        return v;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("socket closed before `{kind}`: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for `{kind}`"))
}

#[tokio::test]
#[ignore = "requires DATABASE_URL + REDIS_URL; see module docs"]
async fn cross_instance_scoring_is_correct() {
    let (Ok(db), Ok(redis)) = (std::env::var("DATABASE_URL"), std::env::var("REDIS_URL")) else {
        eprintln!("skipping: set DATABASE_URL and REDIS_URL to run");
        return;
    };

    let auth = Arc::new(Auth::generate());
    let a = instance(&db, &redis, auth.clone()).await;
    let b = instance(&db, &redis, auth.clone()).await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session = format!("pg-{nanos}");

    let host_token = auth
        .mint(
            &session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let p_token = auth
        .mint(
            &session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    // Host on A, participant on B.
    let (mut host, _) = connect_async(format!("ws://{a}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{b}/ws/{session}?token={p_token}&name=Alice"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Host pushes the question on A.
    host.send(Message::text(
        r#"{"type":"push_question","question":{"id":"q1","text":"x?","choices":["a","b"],"correct_choices":[1],"source_section_ids":["s1"]}}"#
            .to_string(),
    ))
    .await
    .unwrap();

    // Participant on B receives it and answers correctly (recorded in shared Postgres).
    recv_until(&mut p1, "question_opened").await;
    p1.send(Message::text(
        r#"{"type":"submit_answer","question_id":"q1","choices":[1]}"#.to_string(),
    ))
    .await
    .unwrap();

    // The host (on A) sees the answer_received, then reveals — reading shared
    // state, so it MUST score p1 even though p1 answered on B.
    recv_until(&mut host, "answer_received").await;
    host.send(Message::text(r#"{"type":"reveal"}"#.to_string()))
        .await
        .unwrap();
    let rev = recv_until(&mut host, "answers_revealed").await;

    assert_eq!(rev["leaderboard"][0]["participant_id"], "p1");
    assert!(
        rev["leaderboard"][0]["score"].as_u64().unwrap() >= 500,
        "a reveal on instance A must score the answer made on instance B"
    );
    eprintln!("cross-instance scoring correct: A revealed B's answer ✅");
}
