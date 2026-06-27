//! Multi-instance proof: with Redis fanout, a host event pushed on instance A
//! reaches a participant connected to a *different* instance B. Requires Redis;
//! ignored by default. Run with:
//!
//! ```text
//! docker run --rm -d -p 6379:6379 redis
//! REDIS_URL=redis://127.0.0.1/ cargo test --test redis_multi_instance -- --ignored --nocapture
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_server::auth::{Auth, Capability};
use presto_server::redis_fanout::RedisFanout;
use presto_server::store::InMemorySessionStore;
use presto_server::{AppState, app};

async fn instance(url: &str, auth: Arc<Auth>) -> SocketAddr {
    let fanout = Arc::new(RedisFanout::connect(url).await.expect("connect redis"));
    let state = AppState {
        store: Arc::new(InMemorySessionStore::new()),
        fanout,
        auth,
        quiz: Arc::new(presto_server::quiz::FixtureQuizSource),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    addr
}

#[tokio::test]
#[ignore = "requires REDIS_URL; see module docs"]
async fn fanout_crosses_instances_via_redis() {
    let Ok(url) = std::env::var("REDIS_URL") else {
        eprintln!("skipping fanout_crosses_instances_via_redis: set REDIS_URL to run");
        return;
    };

    // Both instances share the same Biscuit key (as a real deployment would).
    let auth = Arc::new(Auth::generate());
    let a = instance(&url, auth.clone()).await;
    let b = instance(&url, auth.clone()).await;
    let session = format!("mi-{}", std::process::id());

    let host_token = auth
        .mint(&session, "host", Capability::Host, Duration::from_secs(600))
        .unwrap();
    let p_token = auth
        .mint(
            &session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
        )
        .unwrap();

    // Host on instance A, participant on instance B.
    let (mut host, _) = connect_async(format!("ws://{a}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{b}/ws/{session}?token={p_token}"))
        .await
        .unwrap();

    // Let B's per-session Redis subscriber finish subscribing before we publish.
    tokio::time::sleep(Duration::from_millis(400)).await;

    host.send(Message::text(
        r#"{"type":"push_question","question":{"id":"q1","text":"x?","choices":["a","b"],"correct_choice":1}}"#
            .to_string(),
    ))
    .await
    .unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match p1.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["type"] == "question_opened" {
                        return v;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("participant socket closed before the question: {other:?}"),
            }
        }
    })
    .await
    .expect("a question pushed on instance A must reach instance B within 5s");

    assert_eq!(got["question"]["id"], "q1");
    eprintln!("multi-instance: question pushed on A reached B via Redis ✅");
}
