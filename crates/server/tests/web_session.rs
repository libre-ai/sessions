//! The web client's HTTP surface end-to-end: `POST /sessions` mints a host
//! token, `POST /sessions/{id}/participants` mints a participant token, and the
//! resulting tokens drive a full live round over the WebSocket.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_server::auth::Auth;
use presto_server::{AppState, app};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn spawn() -> SocketAddr {
    let state = AppState::in_memory(Arc::new(Auth::generate()));
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
async fn joining_an_unknown_session_is_rejected() {
    let addr = spawn().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/sessions/NOPE42/participants"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_session_api_drives_a_full_round() {
    let addr = spawn().await;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // Host creates a session.
    let created: Value = http
        .post(format!("{base}/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = created["data"]["session_id"].as_str().unwrap().to_string();
    let host_token = created["data"]["host_token"].as_str().unwrap().to_string();
    assert!(
        created["data"]["join_url"]
            .as_str()
            .unwrap()
            .contains(&session_id),
        "join_url carries the session code"
    );

    // A participant joins and gets a token.
    let joined: Value = http
        .post(format!("{base}/sessions/{session_id}/participants"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let p_token = joined["data"]["participant_token"]
        .as_str()
        .unwrap()
        .to_string();

    // Both connect; the host opens a (fixture) question, the participant answers.
    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session_id}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!(
        "ws://{addr}/ws/{session_id}?token={p_token}&name=Alice"
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    host.send(Message::text(
        r#"{"type":"generate_question","query":"general"}"#.to_string(),
    ))
    .await
    .unwrap();
    let q = recv_until(&mut p1, "question_opened").await;
    let qid = q["question"]["id"].as_str().unwrap().to_string();
    assert!(
        q["question"].get("correct_choices").is_none(),
        "the answer must not reach participants"
    );

    p1.send(Message::text(format!(
        r#"{{"type":"submit_answer","question_id":"{qid}","choices":[0]}}"#
    )))
    .await
    .unwrap();
    let ack = recv_until(&mut host, "answer_received").await;
    assert!(ack["participant_id"].as_str().unwrap().starts_with("p-"));

    // Host reveals; the participant sees a leaderboard naming Alice.
    host.send(Message::text(r#"{"type":"reveal"}"#.to_string()))
        .await
        .unwrap();
    let rev = recv_until(&mut p1, "answers_revealed").await;
    assert!(
        rev["leaderboard"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "Alice")
    );
}
