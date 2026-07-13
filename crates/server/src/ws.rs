//! The WebSocket session handler: one task per connection. The connection is
//! authorized by a Biscuit join token BEFORE the upgrade. Client messages drive
//! the [`SessionStore`](crate::store) async operations (in-memory or Postgres),
//! then fan out via the [`Fanout`](crate::fanout) seam (tokio broadcast today,
//! Redis across instances).
//!
//! # Token transport & CSWSH
//!
//! Browsers cannot set headers on a WebSocket, so the token rides in the query
//! string. This is acceptable because: it is a **short-TTL, session-scoped
//! capability** (not a long-lived credential or cookie); deployments serve over
//! **wss://** (TLS encrypts the URL in transit) and must not log WS URLs with
//! their query string. The token being the capability also covers cross-site
//! hijacking (CSWSH): an attacker on another origin cannot connect without the
//! token. Future hardening: pass the token via `Sec-WebSocket-Protocol` (needs a
//! token-charset-safe encoding) and add an optional `ALLOWED_ORIGIN` allowlist.

use std::time::SystemTime;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use presto_core::protocol::{ClientMessage, ServerMessage};

use crate::AppState;
use crate::auth::Claims;

/// Query string for `GET /ws/{session_id}`: the Biscuit join token (carries the
/// participant id + capability) plus an optional display name.
#[derive(Debug, Deserialize)]
pub struct ConnectParams {
    pub token: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    Query(params): Query<ConnectParams>,
    State(state): State<AppState>,
) -> Response {
    let claims = match state
        .auth
        .verify(&params.token, &session_id, SystemTime::now())
    {
        Ok(claims) => claims,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid or expired token").into_response(),
    };
    let name = params.name.unwrap_or_else(|| claims.participant_id.clone());
    ws.on_upgrade(move |socket| handle_socket(socket, session_id, claims, name, state))
}

fn to_text(msg: &ServerMessage) -> Utf8Bytes {
    serde_json::to_string(msg)
        .unwrap_or_else(|_| r#"{"type":"error","reason":"serialize"}"#.to_string())
        .into()
}

async fn drain_pending(
    socket: &mut WebSocket,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
) -> bool {
    while let Ok(msg) = rx.try_recv() {
        if socket.send(Message::Text(to_text(&msg))).await.is_err() {
            return false;
        }
    }
    true
}

async fn handle_socket(
    mut socket: WebSocket,
    session_id: String,
    claims: Claims,
    name: String,
    state: AppState,
) {
    let is_host = claims.capability.is_host();
    let host_id = if is_host {
        claims.participant_id.clone()
    } else {
        String::new()
    };
    let mut rx = state.fanout.subscribe(&session_id).await;

    if state.store.ensure(&session_id, &host_id).await.is_err() {
        return; // backend unavailable
    }

    if !is_host {
        match state
            .store
            .join(&session_id, &claims.participant_id, &name)
            .await
        {
            Ok(count) => {
                state
                    .fanout
                    .publish(
                        &session_id,
                        ServerMessage::Joined {
                            participant_id: claims.participant_id.clone(),
                            participants: count,
                        },
                    )
                    .await;
            }
            Err(_) => return,
        }
    }

    if let Ok(Some(question)) = state.store.snapshot(&session_id).await
        && socket
            .send(Message::Text(to_text(&ServerMessage::QuestionOpened {
                question,
            })))
            .await
            .is_err()
    {
        return;
    }
    if !drain_pending(&mut socket, &mut rx).await {
        return;
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let applied = apply(text.as_str(), &claims, &state, &session_id).await;
                        for msg in applied.broadcasts {
                            state.fanout.publish(&session_id, msg).await;
                        }
                        for reply in applied.replies {
                            if socket.send(Message::Text(to_text(&reply))).await.is_err() {
                                return;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(_)) => {} // ignore binary / ping / pong frames
                    Some(Err(_)) => return,
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(m) => {
                        if socket.send(Message::Text(to_text(&m))).await.is_err() {
                            return;
                        }
                    }
                    None => return,
                }
            }
        }
    }
}

/// The result of applying one client message: messages to fan out to the whole
/// session, and replies to send only on this socket.
struct Applied {
    broadcasts: Vec<ServerMessage>,
    replies: Vec<ServerMessage>,
}

fn broadcast(msg: ServerMessage) -> Applied {
    Applied {
        broadcasts: vec![msg],
        replies: vec![],
    }
}

fn reply(msg: ServerMessage) -> Applied {
    Applied {
        broadcasts: vec![],
        replies: vec![msg],
    }
}

/// Apply one client message against the shared state, returning what to send.
async fn apply(text: &str, claims: &Claims, state: &AppState, session_id: &str) -> Applied {
    let pid = &claims.participant_id;
    let is_host = claims.capability.is_host();

    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => {
            return reply(ServerMessage::Error {
                reason: "malformed message".into(),
            });
        }
    };

    match msg {
        ClientMessage::Join { name } => match state.store.join(session_id, pid, &name).await {
            Ok(count) => broadcast(ServerMessage::Joined {
                participant_id: pid.clone(),
                participants: count,
            }),
            Err(e) => reply(ServerMessage::Error {
                reason: e.client_reason().into(),
            }),
        },
        ClientMessage::SubmitAnswer {
            question_id,
            choices,
        } => match state
            .store
            .submit_answer(session_id, pid, &question_id, choices, now_ms())
            .await
        {
            Ok(()) => broadcast(ServerMessage::AnswerReceived {
                participant_id: pid.clone(),
            }),
            Err(e) => reply(ServerMessage::Error {
                reason: e.client_reason().into(),
            }),
        },
        ClientMessage::PushQuestion { mut question } => {
            if !is_host {
                return reply(host_only());
            }
            // `PushQuestion` is client input. Never trust a client-provided
            // citation marker: only server-side fixture/RAG sources may mark a
            // question as grounded for participants.
            question.citation_validation = None;
            push_question(state, session_id, question).await
        }
        ClientMessage::GenerateQuestion { query } => {
            if !is_host {
                return reply(host_only());
            }
            match state.quiz.next_question(&query).await {
                Some(question) => push_question(state, session_id, question).await,
                None => reply(ServerMessage::Error {
                    reason: "no grounded question for query".into(),
                }),
            }
        }
        ClientMessage::Reveal => {
            if !is_host {
                return reply(host_only());
            }
            match state.store.reveal(session_id).await {
                Ok(r) => broadcast(ServerMessage::AnswersRevealed {
                    correct_choices: r.correct_choices,
                    leaderboard: r.leaderboard,
                    heatmap: r.heatmap,
                }),
                Err(e) => reply(ServerMessage::Error {
                    reason: e.client_reason().into(),
                }),
            }
        }
        ClientMessage::Breakout { section_id } => {
            if !is_host {
                return reply(host_only());
            }
            match state.breakout.breakout(&section_id).await {
                Some(explanation) => broadcast(ServerMessage::BreakoutOpened {
                    section_id,
                    explanation,
                }),
                None => reply(ServerMessage::Error {
                    reason: "no breakout available for that section".into(),
                }),
            }
        }
        ClientMessage::Flashcards => match state.store.mastery(session_id, pid).await {
            Ok(mastery) => {
                // Weak = scored below 60% on a section that was actually answered.
                let weak: Vec<String> = mastery
                    .into_iter()
                    .filter(|m| m.total > 0 && (m.correct as f32) / (m.total as f32) < 0.6)
                    .map(|m| m.section_id)
                    .collect();
                let cards = state.flashcards.deck(&weak).await;
                reply(ServerMessage::FlashcardsReady { cards })
            }
            Err(e) => reply(ServerMessage::Error {
                reason: e.client_reason().into(),
            }),
        },
        ClientMessage::Ping => reply(ServerMessage::Pong),
    }
}

fn host_only() -> ServerMessage {
    ServerMessage::Error {
        reason: "host only".into(),
    }
}

/// Store a question and broadcast its public projection (shared by
/// `PushQuestion` and `GenerateQuestion`; the correct answer is never broadcast).
async fn push_question(
    state: &AppState,
    session_id: &str,
    question: presto_core::protocol::Question,
) -> Applied {
    let public = question.public();
    match state
        .store
        .push_question(session_id, &question, now_ms())
        .await
    {
        Ok(()) => broadcast(ServerMessage::QuestionOpened { question: public }),
        Err(e) => reply(ServerMessage::Error {
            reason: e.client_reason().into(),
        }),
    }
}

/// Server clock in epoch milliseconds, the authority for answer timing.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
