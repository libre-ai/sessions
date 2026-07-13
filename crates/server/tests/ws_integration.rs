//! End-to-end WebSocket proof: Biscuit join tokens gate the upgrade, a host
//! event fans out across connections to a participant, the answer never leaks,
//! and reveal returns a scored leaderboard.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_core::protocol::{Question, QuestionKind};
use presto_server::auth::{Auth, Capability};
use presto_server::store::{InMemorySessionStore, SessionStore, StoreResult};
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
    spawn_server_with_state(state, auth).await
}

async fn spawn_server_with_state(state: AppState, auth: Arc<Auth>) -> Server {
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

struct BlockingSnapshotStore {
    inner: InMemorySessionStore,
    entered: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    resume: Arc<tokio::sync::Notify>,
    paused_once: AtomicBool,
}

impl BlockingSnapshotStore {
    fn new(entered: tokio::sync::oneshot::Sender<()>, resume: Arc<tokio::sync::Notify>) -> Self {
        Self {
            inner: InMemorySessionStore::new(),
            entered: Arc::new(Mutex::new(Some(entered))),
            resume,
            paused_once: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl SessionStore for BlockingSnapshotStore {
    async fn ensure(&self, session_id: &str, host_id: &str) -> StoreResult<()> {
        self.inner.ensure(session_id, host_id).await
    }

    async fn join(&self, session_id: &str, participant_id: &str, name: &str) -> StoreResult<u32> {
        self.inner.join(session_id, participant_id, name).await
    }

    async fn push_question(
        &self,
        session_id: &str,
        question: &presto_core::protocol::Question,
        opened_at_ms: u64,
    ) -> StoreResult<()> {
        self.inner
            .push_question(session_id, question, opened_at_ms)
            .await
    }

    async fn submit_answer(
        &self,
        session_id: &str,
        participant_id: &str,
        question_id: &str,
        choices: Vec<u8>,
        now_ms: u64,
    ) -> StoreResult<()> {
        self.inner
            .submit_answer(session_id, participant_id, question_id, choices, now_ms)
            .await
    }

    async fn snapshot(
        &self,
        session_id: &str,
    ) -> StoreResult<Option<presto_core::protocol::QuestionPublic>> {
        let snap = self.inner.snapshot(session_id).await;
        if !self.paused_once.swap(true, Ordering::SeqCst) {
            if let Some(tx) = self.entered.lock().unwrap().take() {
                let _ = tx.send(());
            }
            self.resume.notified().await;
        }
        snap
    }

    async fn exists(&self, session_id: &str) -> StoreResult<bool> {
        self.inner.exists(session_id).await
    }

    async fn mastery(
        &self,
        session_id: &str,
        participant_id: &str,
    ) -> StoreResult<Vec<presto_server::session::SectionMastery>> {
        self.inner.mastery(session_id, participant_id).await
    }

    async fn reveal(&self, session_id: &str) -> StoreResult<presto_server::session::RevealResult> {
        self.inner.reveal(session_id).await
    }
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
        .mint(
            "sess1",
            "host",
            Capability::Host,
            Duration::from_secs(3600),
            SystemTime::now(),
        )
        .unwrap();
    let p1_token = srv
        .auth
        .mint(
            "sess1",
            "p1",
            Capability::Participant,
            Duration::from_secs(3600),
            SystemTime::now(),
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
        r#"{"type":"push_question","question":{"id":"q1","text":"2+2?","choices":["3","4","5"],"correct_choices":[1],"source_section_ids":["doc1#s2"]}}"#,
    )
    .await;
    let q = recv_until(&mut p1, "question_opened").await;
    assert_eq!(q["question"]["id"], "q1");
    assert!(
        q["question"].get("correct_choices").is_none(),
        "the answer leaked to a participant"
    );

    // p1 answers correctly; the host sees an answer_received.
    send(
        &mut p1,
        r#"{"type":"submit_answer","question_id":"q1","choices":[1]}"#,
    )
    .await;
    let ack = recv_until(&mut host, "answer_received").await;
    assert_eq!(ack["participant_id"], "p1");

    // Host reveals; p1 receives a scored leaderboard + zero confusion (it was right).
    send(&mut host, r#"{"type":"reveal"}"#).await;
    let rev = recv_until(&mut p1, "answers_revealed").await;
    assert_eq!(rev["correct_choices"], serde_json::json!([1]));
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
            SystemTime::now(),
        )
        .unwrap();
    let p_token = srv
        .auth
        .mint(
            "sess-late",
            "p1",
            Capability::Participant,
            Duration::from_secs(3600),
            SystemTime::now(),
        )
        .unwrap();
    let base = format!("ws://{}/ws/sess-late", srv.addr);

    // The host opens a question BEFORE the participant connects.
    let (mut host, _) = connect_async(format!("{base}?token={host_token}"))
        .await
        .unwrap();
    send(
        &mut host,
        r#"{"type":"push_question","question":{"id":"q1","text":"2+2?","choices":["3","4"],"correct_choices":[1],"source_section_ids":["doc1#s2"]}}"#,
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
        q["question"].get("correct_choices").is_none(),
        "the snapshot must not leak the answer"
    );
}

#[tokio::test]
async fn answer_errors_use_stable_codes_without_backend_leakage() {
    let srv = spawn_server().await;
    let host_token = srv
        .auth
        .mint(
            "sess-err",
            "host",
            Capability::Host,
            Duration::from_secs(3600),
            SystemTime::now(),
        )
        .unwrap();
    let p1_token = srv
        .auth
        .mint(
            "sess-err",
            "p1",
            Capability::Participant,
            Duration::from_secs(3600),
            SystemTime::now(),
        )
        .unwrap();
    let p2_token = srv
        .auth
        .mint(
            "sess-err",
            "p2",
            Capability::Participant,
            Duration::from_secs(3600),
            SystemTime::now(),
        )
        .unwrap();
    let base = format!("ws://{}/ws/sess-err", srv.addr);

    let (mut host, _) = connect_async(format!("{base}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("{base}?token={p1_token}&name=Alice"))
        .await
        .unwrap();
    let (mut p2, _) = connect_async(format!("{base}?token={p2_token}&name=Bob"))
        .await
        .unwrap();
    recv_until(&mut p1, "joined").await;
    recv_until(&mut p2, "joined").await;

    send(
        &mut host,
        r#"{"type":"push_question","question":{"id":"q1","text":"2+2?","choices":["3","4"],"correct_choices":[1],"source_section_ids":["doc1#s2"],"timer_sec":0}}"#,
    )
    .await;
    recv_until(&mut p1, "question_opened").await;
    recv_until(&mut p2, "question_opened").await;

    send(
        &mut p1,
        r#"{"type":"submit_answer","question_id":"wrong","choices":[1]}"#,
    )
    .await;
    let err = recv_until(&mut p1, "error").await;
    assert_eq!(err["reason"], "wrong_question");

    send(
        &mut p1,
        r#"{"type":"submit_answer","question_id":"q1","choices":[]}"#,
    )
    .await;
    let err = recv_until(&mut p1, "error").await;
    assert_eq!(err["reason"], "invalid_answer");

    send(
        &mut p1,
        r#"{"type":"submit_answer","question_id":"q1","choices":[1]}"#,
    )
    .await;
    recv_until(&mut host, "answer_received").await;

    send(
        &mut p1,
        r#"{"type":"submit_answer","question_id":"q1","choices":[1]}"#,
    )
    .await;
    let err = recv_until(&mut p1, "error").await;
    assert_eq!(err["reason"], "already_answered");

    tokio::time::sleep(Duration::from_millis(1600)).await;
    send(
        &mut p2,
        r#"{"type":"submit_answer","question_id":"q1","choices":[1]}"#,
    )
    .await;
    let err = recv_until(&mut p2, "error").await;
    assert_eq!(err["reason"], "answer_closed");
}

#[tokio::test]
async fn late_joiner_does_not_emit_a_stale_question_when_reveal_wins_the_race() {
    let auth = Arc::new(Auth::generate());
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let resume = Arc::new(tokio::sync::Notify::new());
    let store = Arc::new(BlockingSnapshotStore::new(entered_tx, resume.clone()));
    let mut state = AppState::in_memory(auth.clone());
    state.store = store.clone();
    let srv = spawn_server_with_state(state, auth.clone()).await;

    store.ensure("sess-race", "host").await.unwrap();
    store
        .push_question(
            "sess-race",
            &Question {
                id: "q1".into(),
                text: "2+2?".into(),
                kind: QuestionKind::Single,
                choices: vec!["3".into(), "4".into()],
                correct_choices: vec![1],
                source_section_ids: vec!["doc1#s2".into()],
                citation_validation: None,
                timer_sec: 30,
            },
            0,
        )
        .await
        .unwrap();

    let p_token = srv
        .auth
        .mint(
            "sess-race",
            "p1",
            Capability::Participant,
            Duration::from_secs(3600),
            SystemTime::now(),
        )
        .unwrap();
    let base = format!("ws://{}/ws/sess-race", srv.addr);

    let (mut p1, _) = connect_async(format!("{base}?token={p_token}"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), entered_rx)
        .await
        .unwrap()
        .unwrap();

    store.reveal("sess-race").await.unwrap();
    resume.notify_one();

    let stale = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            match p1.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["type"] == "question_opened" {
                        panic!("stale question_opened leaked after reveal");
                    }
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return,
            }
        }
    })
    .await;
    assert!(
        stale.is_err(),
        "question_opened should not reappear after reveal"
    );
}
