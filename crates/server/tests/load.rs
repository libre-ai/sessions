//! Single-instance scale proof: 200 participants receive every pushed question
//! with p99 delivery latency under 200 ms and zero loss. Ignored by default —
//! run with: `cargo test --release --test load -- --ignored --nocapture`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_core::fixtures::sample_quiz;
use presto_core::protocol::ClientMessage;
use presto_server::auth::{Auth, Capability};
use presto_server::registry::SessionRegistry;
use presto_server::{AppState, app};

const PARTICIPANTS: usize = 200;

async fn spawn() -> (SocketAddr, Arc<Auth>) {
    let auth = Arc::new(Auth::generate());
    let state = AppState {
        registry: SessionRegistry::new(),
        auth: auth.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (addr, auth)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test — run with: cargo test --release --test load -- --ignored --nocapture"]
async fn sustains_200_participants_under_200ms_p99() {
    let (addr, auth) = spawn().await;
    let session = "load";
    let quiz = sample_quiz();
    let questions = quiz.len();
    let base = format!("ws://{addr}/ws/{session}");

    // Host: split the socket so a task drains its inbound (broadcasts) while we
    // send commands — otherwise TCP backpressure on the host's unread inbound
    // would stall the server's host loop.
    let host_token = auth
        .mint(session, "host", Capability::Host, Duration::from_secs(3600))
        .unwrap();
    let (host, _) = connect_async(format!("{base}?token={host_token}"))
        .await
        .unwrap();
    let (mut host_tx, mut host_rx) = host.split();
    let host_drain = tokio::spawn(async move { while host_rx.next().await.is_some() {} });

    let push_times: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let (tx, mut rx) = mpsc::unbounded_channel::<u128>();

    let mut handles = Vec::with_capacity(PARTICIPANTS);
    for i in 0..PARTICIPANTS {
        let pid = format!("p{i}");
        let token = auth
            .mint(
                session,
                &pid,
                Capability::Participant,
                Duration::from_secs(3600),
            )
            .unwrap();
        let (mut ws, _) = connect_async(format!("{base}?token={token}"))
            .await
            .unwrap();
        let push_times = push_times.clone();
        let tx = tx.clone();
        handles.push(tokio::spawn(async move {
            let mut seen = 0usize;
            let run = async {
                while seen < questions {
                    match ws.next().await {
                        Some(Ok(Message::Text(t))) => {
                            let v: Value = serde_json::from_str(t.as_str()).unwrap();
                            if v["type"] == "question_opened" {
                                let qid = v["question"]["id"].as_str().unwrap().to_string();
                                if let Some(p) = push_times.lock().unwrap().get(&qid).copied() {
                                    let _ = tx.send(p.elapsed().as_micros());
                                }
                                seen += 1;
                                let ans = ClientMessage::SubmitAnswer {
                                    question_id: qid,
                                    choice: 0,
                                    elapsed_ms: 500,
                                };
                                let _ = ws
                                    .send(Message::text(serde_json::to_string(&ans).unwrap()))
                                    .await;
                            }
                        }
                        Some(Ok(_)) => {}
                        _ => break,
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(20), run).await;
        }));
    }

    // Let every participant finish subscribing before the first push.
    tokio::time::sleep(Duration::from_millis(300)).await;

    for question in &quiz {
        let msg = ClientMessage::PushQuestion {
            question: question.clone(),
        };
        push_times
            .lock()
            .unwrap()
            .insert(question.id.clone(), Instant::now());
        host_tx
            .send(Message::text(serde_json::to_string(&msg).unwrap()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(700)).await;
        host_tx
            .send(Message::text(r#"{"type":"reveal"}"#.to_string()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    for h in handles {
        let _ = h.await;
    }
    drop(tx);
    host_drain.abort();

    let mut samples: Vec<u128> = Vec::new();
    while let Some(s) = rx.recv().await {
        samples.push(s);
    }
    samples.sort_unstable();
    let pct = |p: f64| -> u128 {
        if samples.is_empty() {
            0
        } else {
            samples[(((samples.len() - 1) as f64) * p).round() as usize]
        }
    };
    let expected = PARTICIPANTS * questions;
    eprintln!(
        "load: expected {expected} deliveries, got {} | p50={}µs p95={}µs p99={}µs",
        samples.len(),
        pct(0.50),
        pct(0.95),
        pct(0.99),
    );

    assert_eq!(
        samples.len(),
        expected,
        "zero loss: every question must reach every participant"
    );
    let p99_ms = pct(0.99) as f64 / 1000.0;
    assert!(p99_ms < 200.0, "p99 delivery {p99_ms:.1}ms must be < 200ms");
}
