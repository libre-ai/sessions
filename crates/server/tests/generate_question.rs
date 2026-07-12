//! Wiring the RAG pipeline into a live session: a host `GenerateQuestion`
//! produces a grounded question (from the [`QuizSource`]) and pushes it into the
//! session, where participants receive it via the existing fanout. A fake
//! `QuizSource` stands in for the retrieve → generate → verify pipeline (whose
//! logic is tested in `presto-rag`); host-only authorization is enforced here.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use presto_core::protocol::{CitationValidation, Question};
use presto_rag::corpus::{CorpusError, RetrievalScope, Retrieved, Retriever};
use presto_rag::provider::{AiError, AiProvider};
use presto_server::auth::{Auth, Capability};
use presto_server::fanout::BroadcastFanout;
use presto_server::quiz::{QuizSource, RagQuizSource};
use presto_server::store::InMemorySessionStore;
use presto_server::{AppState, app};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Stands in for the RAG pipeline: returns one grounded, source-cited question.
struct OneQuestionQuiz;

#[async_trait]
impl QuizSource for OneQuestionQuiz {
    async fn next_question(&self, query: &str) -> Option<Question> {
        Some(Question {
            id: "gen:doc#p0".into(),
            text: format!("What about {query}?"),
            kind: presto_core::protocol::QuestionKind::Single,
            choices: vec!["yes".into(), "no".into()],
            correct_choices: vec![0],
            source_section_ids: vec!["doc#p0".into()],
            citation_validation: Some(CitationValidation::verified(1)),
            timer_sec: 20,
        })
    }
}

/// QuizSource that returns None: simulates corpus retrieval failure or no suitable grounding found.
struct NoQuestionQuiz;

#[async_trait]
impl QuizSource for NoQuestionQuiz {
    async fn next_question(&self, _query: &str) -> Option<Question> {
        None
    }
}

/// Mock retriever: always returns one chunk of text for testing.
struct MockRetriever;

#[async_trait]
impl Retriever for MockRetriever {
    async fn retrieve(
        &self,
        _scope: &RetrievalScope,
        _query: &str,
        _k: usize,
        _provider: &dyn AiProvider,
    ) -> Result<Vec<Retrieved>, CorpusError> {
        Ok(vec![Retrieved {
            source_section_id: "doc#p0".into(),
            text: "The sky is blue.".into(),
            distance: 0.0,
        }])
    }

    async fn fetch_section(
        &self,
        _scope: &RetrievalScope,
        section_id: &str,
    ) -> Result<Option<presto_rag::corpus::Chunk>, CorpusError> {
        Ok((section_id == "doc#p0").then(|| presto_rag::corpus::Chunk {
            source_section_id: "doc#p0".into(),
            text: "The sky is blue.".into(),
        }))
    }
}

/// Mock AI provider: simulates generation and structured verification evidence.
/// `verifier_supports` controls whether the exact lexical gate passes.
struct MockAiProvider {
    verifier_supports: bool,
}

#[async_trait]
impl AiProvider for MockAiProvider {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        Ok(vec![])
    }

    async fn complete(&self, system: &str, _user: &str) -> Result<String, AiError> {
        if system.contains("grounding checker") {
            // Verifier response: gate pass/fail based on verifier_supports flag.
            Ok(if self.verifier_supports {
                "{\"supported\":true,\"reason\":\"exact\",\
                 \"evidence\":{\"source_section_id\":\"doc#p0\",\
                 \"exact_quote\":\"The sky is blue.\"}}"
                    .into()
            } else {
                "{\"supported\":false,\"reason\":\"absent\",\"evidence\":null}".into()
            })
        } else {
            // Generator response: always generate a valid question.
            Ok(
                "{\"text\":\"What is the color of the sky?\",\"choices\":[\"blue\",\"red\",\"green\",\"yellow\"],\"correct_choices\":[0]}"
                    .to_string(),
            )
        }
    }
}

async fn spawn(quiz: Arc<dyn QuizSource>, auth: Arc<Auth>) -> std::net::SocketAddr {
    let state = AppState {
        store: Arc::new(InMemorySessionStore::new()),
        fanout: Arc::new(BroadcastFanout::new()),
        owner_auth: Arc::new(presto_server::owner_auth::OwnerAuth::disabled(auth.clone())),
        auth,
        quiz,
        breakout: Arc::new(presto_server::quiz::FixtureBreakoutSource),
        flashcards: Arc::new(presto_server::quiz::FixtureFlashcardSource),
        ingestor: Arc::new(presto_server::quiz::FixtureIngestor),
        session_rate: Arc::new(presto_server::ratelimit::TokenBucket::new(1000.0, 1000.0)),
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
async fn host_generate_question_reaches_participants() {
    let auth = Arc::new(Auth::generate());
    let addr = spawn(Arc::new(OneQuestionQuiz), auth.clone()).await;
    let session = "s-gen";
    let host_token = auth
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{addr}/ws/{session}?token={p_token}"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    host.send(Message::text(
        r#"{"type":"generate_question","query":"rust"}"#.to_string(),
    ))
    .await
    .unwrap();

    let opened = recv_until(&mut p1, "question_opened").await;
    assert_eq!(opened["question"]["text"], "What about rust?");
    // The public projection must not leak the correct answer to participants.
    assert!(opened["question"].get("correct_choices").is_none());
}

#[tokio::test]
async fn participant_cannot_generate_question() {
    let auth = Arc::new(Auth::generate());
    let addr = spawn(Arc::new(OneQuestionQuiz), auth.clone()).await;
    let session = "s-gen2";
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut p1, _) = connect_async(format!("ws://{addr}/ws/{session}?token={p_token}"))
        .await
        .unwrap();

    p1.send(Message::text(
        r#"{"type":"generate_question","query":"rust"}"#.to_string(),
    ))
    .await
    .unwrap();

    let err = recv_until(&mut p1, "error").await;
    assert_eq!(err["reason"], "host only");
}

#[tokio::test]
async fn host_generate_question_with_no_grounding_returns_error() {
    let auth = Arc::new(Auth::generate());
    let addr = spawn(Arc::new(NoQuestionQuiz), auth.clone()).await;
    let session = "s-gen-none";
    let host_token = auth
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    host.send(Message::text(
        r#"{"type":"generate_question","query":"unretrievable"}"#.to_string(),
    ))
    .await
    .unwrap();

    let err = recv_until(&mut host, "error").await;
    assert_eq!(err["reason"], "no grounded question for query");
}

#[tokio::test]
async fn rag_pipeline_verifier_gate_drops_ungrounded_question() {
    // End-to-end test: the RAG pipeline's grounding verifier gate must prevent
    // ungrounded questions from reaching participants.
    // When verify_grounding() returns supported=false, the question is dropped,
    // the host receives an error reply, and no broadcast occurs.
    let auth = Arc::new(Auth::generate());

    let retriever = Arc::new(MockRetriever);
    let provider = Arc::new(MockAiProvider {
        verifier_supports: false, // Gate the question: verification fails.
    });
    let quiz = Arc::new(RagQuizSource::new(retriever, provider));

    let addr = spawn(quiz, auth.clone()).await;
    let session = "s-gen-verifier-gate";

    let host_token = auth
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{addr}/ws/{session}?token={p_token}"))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(150)).await;

    host.send(Message::text(
        r#"{"type":"generate_question","query":"sky color"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Host must receive an error (question failed grounding verification).
    let err = recv_until(&mut host, "error").await;
    assert_eq!(err["reason"], "no grounded question for query");

    // Participant must NOT receive a question_opened broadcast.
    // Verify by expecting a timeout: if question_opened is received, it fails.
    let no_broadcast_fut = async {
        loop {
            match p1.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["type"] == "question_opened" {
                        panic!(
                            "verifier gate failed: ungrounded question was broadcast to participants"
                        );
                    }
                }
                Some(Ok(_)) => {}
                _ => break,
            }
        }
    };

    // Allow a short window to confirm no broadcast arrives.
    tokio::time::timeout(Duration::from_millis(500), no_broadcast_fut)
        .await
        .ok(); // Timeout is expected; socket may close or no message arrives.
}

#[tokio::test]
async fn rag_pipeline_accepts_grounded_question() {
    // Positive case: supported=true plus matching exact evidence passes the gate
    // and reaches participants as expected.
    let auth = Arc::new(Auth::generate());

    let retriever = Arc::new(MockRetriever);
    let provider = Arc::new(MockAiProvider {
        verifier_supports: true, // Gate passes: verification succeeds.
    });
    let quiz = Arc::new(RagQuizSource::new(retriever, provider));

    let addr = spawn(quiz, auth.clone()).await;
    let session = "s-gen-accept";

    let host_token = auth
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{addr}/ws/{session}?token={p_token}"))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(150)).await;

    host.send(Message::text(
        r#"{"type":"generate_question","query":"sky color"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Participant must receive the grounded question broadcast.
    let opened = recv_until(&mut p1, "question_opened").await;
    assert_eq!(opened["question"]["text"], "What is the color of the sky?");
    assert!(opened["question"].get("correct_choices").is_none());
}

#[tokio::test]
async fn generated_question_preserves_correct_choice_through_reveal() {
    // Regression test: verify that when a host uses GenerateQuestion, the server
    // preserves the full Question (including correct_choice) for later reveal.
    // Previously, if the question was only partially stored, the reveal would fail
    // or return incorrect data.
    let auth = Arc::new(Auth::generate());
    let addr = spawn(Arc::new(OneQuestionQuiz), auth.clone()).await;
    let session = "s-gen-reveal";
    let host_token = auth
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!(
        "ws://{addr}/ws/{session}?token={p_token}&name=Alice"
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Host generates a question from the corpus.
    host.send(Message::text(
        r#"{"type":"generate_question","query":"rust"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Participant receives the question without the correct answer.
    let opened = recv_until(&mut p1, "question_opened").await;
    let q_id = opened["question"]["id"].as_str().unwrap();
    assert_eq!(opened["question"]["text"], "What about rust?");
    assert!(opened["question"].get("correct_choices").is_none());

    // Participant answers (the OneQuestionQuiz has correct_choice=0, so answering 0 is correct).
    p1.send(Message::text(format!(
        r#"{{"type":"submit_answer","question_id":"{}","choices":[0]}}"#,
        q_id
    )))
    .await
    .unwrap();

    // Host sees the answer_received broadcast.
    let ack = recv_until(&mut host, "answer_received").await;
    assert_eq!(ack["participant_id"], "p1");

    // Host reveals the answers; server must have preserved the correct_choice from generation.
    host.send(Message::text(r#"{"type":"reveal"}"#.to_string()))
        .await
        .unwrap();

    let revealed = recv_until(&mut p1, "answers_revealed").await;
    // The core assertion: the server still knows the correct_choice.
    assert_eq!(revealed["correct_choices"], serde_json::json!([0]));
    // Verify the leaderboard is populated.
    assert_eq!(revealed["leaderboard"][0]["participant_id"], "p1");
    assert!(revealed["leaderboard"][0]["score"].as_u64().unwrap() >= 500);
}

#[tokio::test]
async fn host_breakout_reaches_participants() {
    let auth = Arc::new(Auth::generate());
    let addr = spawn(Arc::new(OneQuestionQuiz), auth.clone()).await;
    let session = "s-breakout";
    let host_token = auth
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{addr}/ws/{session}?token={p_token}"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // The host opens a grounded breakout for a confused section; participants get it.
    host.send(Message::text(
        r#"{"type":"breakout","section_id":"doc#p0"}"#.to_string(),
    ))
    .await
    .unwrap();
    let bo = recv_until(&mut p1, "breakout_opened").await;
    assert_eq!(bo["section_id"], "doc#p0");
    assert!(bo["explanation"].as_str().unwrap().contains("doc#p0"));
}

#[tokio::test]
async fn participant_cannot_open_a_breakout() {
    let auth = Arc::new(Auth::generate());
    let addr = spawn(Arc::new(OneQuestionQuiz), auth.clone()).await;
    let session = "s-breakout2";
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{addr}/ws/{session}?token={p_token}"))
        .await
        .unwrap();
    p1.send(Message::text(
        r#"{"type":"breakout","section_id":"doc#p0"}"#.to_string(),
    ))
    .await
    .unwrap();
    let err = recv_until(&mut p1, "error").await;
    assert_eq!(err["reason"], "host only");
}

#[tokio::test]
async fn participant_gets_flashcards_for_weak_sections() {
    let auth = Arc::new(Auth::generate());
    let addr = spawn(Arc::new(OneQuestionQuiz), auth.clone()).await;
    let session = "s-flash";
    let host_token = auth
        .mint(
            session,
            "host",
            Capability::Host,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();
    let p_token = auth
        .mint(
            session,
            "p1",
            Capability::Participant,
            Duration::from_secs(600),
            SystemTime::now(),
        )
        .unwrap();

    let (mut host, _) = connect_async(format!("ws://{addr}/ws/{session}?token={host_token}"))
        .await
        .unwrap();
    let (mut p1, _) = connect_async(format!("ws://{addr}/ws/{session}?token={p_token}"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Host opens a question on section doc#p0; the participant answers wrong.
    host.send(Message::text(
        r#"{"type":"generate_question","query":"x"}"#.to_string(),
    ))
    .await
    .unwrap();
    let q = recv_until(&mut p1, "question_opened").await;
    let qid = q["question"]["id"].as_str().unwrap().to_string();
    p1.send(Message::text(format!(
        r#"{{"type":"submit_answer","question_id":"{qid}","choices":[1]}}"#
    )))
    .await
    .unwrap();
    recv_until(&mut host, "answer_received").await;

    // Reveal accumulates mastery (0/1 on doc#p0 → weak).
    host.send(Message::text(r#"{"type":"reveal"}"#.to_string()))
        .await
        .unwrap();
    recv_until(&mut p1, "answers_revealed").await;

    // The participant requests their spaced-repetition deck.
    p1.send(Message::text(r#"{"type":"flashcards"}"#.to_string()))
        .await
        .unwrap();
    let deck = recv_until(&mut p1, "flashcards_ready").await;
    let cards = deck["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1, "one weak section → one card");
    assert_eq!(cards[0]["section_id"], "doc#p0");
    assert_eq!(cards[0]["ease_factor"], 2.5);
}
