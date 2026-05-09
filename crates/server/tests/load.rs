//! Single-instance scale proof: 200 participants receive every pushed question
//! with p99 delivery latency under 200 ms and zero loss. Ignored by default —
//! run with: `cargo test --release --test load -- --ignored --nocapture`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_core::fixtures::sample_quiz;
use presto_core::protocol::ClientMessage;
use presto_server::auth::{Auth, Capability};
use presto_server::{AppState, app};

const PARTICIPANTS: usize = 200;

async fn spawn() -> (SocketAddr, Arc<Auth>) {
    let auth = Arc::new(Auth::generate());
    let state = AppState::in_memory(auth.clone());
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
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(3600),
            SystemTime::now(),
        )
        .unwrap();
    let (host, _) = connect_async(format!("{base}?token={host_token}"))
        .await
        .unwrap();
    let (mut host_tx, mut host_rx) = host.split();
    let host_drain = tokio::spawn(async move { while host_rx.next().await.is_some() {} });

    let push_times: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let (tx, mut rx) = mpsc::unbounded_channel::<u128>();
    // Reveal latency (§3 SLO): reveal scores all 200 participants under the
    // session lock before broadcasting — a distinct cost from a plain push. Track
    // the most recent reveal-send instant + a channel of per-participant samples.
    let reveal_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let (rtx, mut rrx) = mpsc::unbounded_channel::<u128>();

    let mut handles = Vec::with_capacity(PARTICIPANTS);
    for i in 0..PARTICIPANTS {
        let pid = format!("p{i}");
        let token = auth
            .mint(
                session,
                &pid,
                Capability::Participant,
                Duration::from_secs(3600),
                SystemTime::now(),
            )
            .unwrap();
        let (mut ws, _) = connect_async(format!("{base}?token={token}"))
            .await
            .unwrap();
        let push_times = push_times.clone();
        let tx = tx.clone();
        let reveal_at = reveal_at.clone();
        let rtx = rtx.clone();
        handles.push(tokio::spawn(async move {
            // Wait until every reveal is seen (reveals follow questions, so this
            // also covers every question delivery).
            let mut reveals = 0usize;
            let run = async {
                while reveals < questions {
                    match ws.next().await {
                        Some(Ok(Message::Text(t))) => {
                            let v: Value = serde_json::from_str(t.as_str()).unwrap();
                            match v["type"].as_str() {
                                Some("question_opened") => {
                                    let qid = v["question"]["id"].as_str().unwrap().to_string();
                                    if let Some(p) = push_times.lock().unwrap().get(&qid).copied() {
                                        let _ = tx.send(p.elapsed().as_micros());
                                    }
                                    let ans = ClientMessage::SubmitAnswer {
                                        question_id: qid,
                                        choices: vec![0],
                                    };
                                    let _ = ws
                                        .send(Message::text(serde_json::to_string(&ans).unwrap()))
                                        .await;
                                }
                                Some("answers_revealed") => {
                                    if let Some(at) = *reveal_at.lock().unwrap() {
                                        let _ = rtx.send(at.elapsed().as_micros());
                                    }
                                    reveals += 1;
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(_)) => {}
                        _ => break,
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(30), run).await;
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
        *reveal_at.lock().unwrap() = Some(Instant::now());
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
    drop(rtx);
    host_drain.abort();

    let mut samples: Vec<u128> = Vec::new();
    while let Some(s) = rx.recv().await {
        samples.push(s);
    }
    samples.sort_unstable();
    let mut reveal_samples: Vec<u128> = Vec::new();
    while let Some(s) = rrx.recv().await {
        reveal_samples.push(s);
    }
    reveal_samples.sort_unstable();

    let expected = PARTICIPANTS * questions;
    eprintln!(
        "load delivery: got {}/{expected} | p50={}µs p95={}µs p99={}µs",
        samples.len(),
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        percentile(&samples, 0.99),
    );
    eprintln!(
        "load reveal:   got {}/{expected} | p50={}µs p95={}µs p99={}µs",
        reveal_samples.len(),
        percentile(&reveal_samples, 0.50),
        percentile(&reveal_samples, 0.95),
        percentile(&reveal_samples, 0.99),
    );

    // §3 SLO — delivery: zero loss, p99 < 200ms.
    assert_eq!(
        samples.len(),
        expected,
        "zero loss: every question must reach every participant"
    );
    let p99_ms = percentile(&samples, 0.99) as f64 / 1000.0;
    assert!(p99_ms < 200.0, "p99 delivery {p99_ms:.1}ms must be < 200ms");

    // §3 SLO — reveal: every reveal reaches everyone, p99 < 500ms (scoring 200
    // participants under the lock + broadcast).
    assert_eq!(
        reveal_samples.len(),
        expected,
        "every reveal must reach every participant"
    );
    let reveal_p99_ms = percentile(&reveal_samples, 0.99) as f64 / 1000.0;
    assert!(
        reveal_p99_ms < 500.0,
        "p99 reveal {reveal_p99_ms:.1}ms must be < 500ms"
    );
}

/// The `p`-quantile of a pre-sorted sample slice (0 if empty).
fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        0
    } else {
        sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
    }
}
