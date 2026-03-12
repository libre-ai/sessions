//! End-to-end WebSocket proof: Biscuit join tokens gate the upgrade, a host
//! event fans out across connections to a participant, the answer never leaks,
//! and reveal returns a scored leaderboard.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_server::auth::{Auth, Capability};
use presto_server::{AppState, app};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Server {
    addr: SocketAddr,
    auth: Arc<Auth>,
}

async fn spawn_server() -> Server {
    let auth = Arc::new(Auth::generate());
    let state = AppState::in_memory(auth.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    Server { addr, auth }
}

async fn send(ws: &mut Ws, payload: &str) {
    ws.send(Message::text(payload.to_string())).await.unwrap();
}

/// Read frames until one whose `"type"` equals `kind`; panics on timeout.
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
                Some(Ok(_)) => continue,
                other => panic!("socket closed/error before `{kind}`: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for `{kind}`"))
}

#[tokio::test]
async fn invalid_token_is_rejected_before_upgrade() {
    let srv = spawn_server().await;
    let url = format!("ws://{}/ws/sess1?token=not-a-biscuit", srv.addr);
    assert!(
        connect_async(url).await.is_err(),
        "an unauthenticated upgrade must be refused"
    );
}

#[tokio::test]
async fn live_round_fans_out_host_events_to_participants() {
    let srv = spawn_server().await;
    let host_token = srv
        .auth
        .mint("sess1", "host", Capability::Host, Duration::from_secs(3600))
        .unwrap();
    let p1_token = srv
        .auth
        .mint(
            "sess1",
            "p1",
            Capability::Participant,
            Duration::from_secs(3600),
        )
        .unwrap();
    let base = format!("ws://{}/ws/sess1", srv.addr);

    let (mut host, _) = connect_async(format!("{base}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("{base}?token={p1_token}&name=Alice"))
        .await
        .unwrap();

    // p1 auto-joined; drain the `joined` broadcast.
    recv_until(&mut p1, "joined").await;

    // Host pushes a question — the correct answer must NOT reach participants.
    send(
        &mut host,
        r#"{"type":"push_question","question":{"id":"q1","text":"2+2?","choices":["3","4","5"],"correct_choice":1,"source_section_ids":["doc1#s2"]}}"#,
    )
    .await;
    let q = recv_until(&mut p1, "question_opened").await;
    assert_eq!(q["question"]["id"], "q1");
    assert!(
        q["question"].get("correct_choice").is_none(),
        "the answer leaked to a participant"
    );

    // p1 answers correctly; the host sees an answer_received.
    send(
        &mut p1,
        r#"{"type":"submit_answer","question_id":"q1","choice":1}"#,
    )
    .await;
    let ack = recv_until(&mut host, "answer_received").await;
    assert_eq!(ack["participant_id"], "p1");

    // Host reveals; p1 receives a scored leaderboard + zero confusion (it was right).
    send(&mut host, r#"{"type":"reveal"}"#).await;
    let rev = recv_until(&mut p1, "answers_revealed").await;
    assert_eq!(rev["correct_choice"], 1);
    assert_eq!(rev["leaderboard"][0]["participant_id"], "p1");
    assert!(rev["leaderboard"][0]["score"].as_u64().unwrap() >= 500);
    assert!(rev["heatmap"]["doc1#s2"].as_f64().unwrap().abs() < 1e-6);
}

#[tokio::test]
async fn late_joiner_receives_the_open_question() {
    let srv = spawn_server().await;
    let host_token = srv
        .auth
        .mint(
            "sess-late",
            "host",
            Capability::Host,
            Duration::from_secs(3600),
        )
        .unwrap();
    let p_token = srv
        .auth
        .mint(
            "sess-late",
            "p1",
            Capability::Participant,
            Duration::from_secs(3600),
        )
        .unwrap();
    let base = format!("ws://{}/ws/sess-late", srv.addr);

    // The host opens a question BEFORE the participant connects.
    let (mut host, _) = connect_async(format!("{base}?token={host_token}"))
        .await
        .unwrap();
    send(
        &mut host,
        r#"{"type":"push_question","question":{"id":"q1","text":"2+2?","choices":["3","4"],"correct_choice":1,"source_section_ids":["doc1#s2"]}}"#,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A participant joining mid-question receives the open question immediately
    // (snapshot on connect), rather than waiting for the next broadcast.
    let (mut p1, _) = connect_async(format!("{base}?token={p_token}"))
        .await
        .unwrap();
    let q = recv_until(&mut p1, "question_opened").await;
    assert_eq!(q["question"]["id"], "q1");
    assert!(
        q["question"].get("correct_choice").is_none(),
        "the snapshot must not leak the answer"
    );
}
