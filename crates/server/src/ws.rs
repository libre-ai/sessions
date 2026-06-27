//! The WebSocket session handler: one task per connection. The connection is
//! authorized by a Biscuit join token BEFORE the upgrade. Client messages drive
//! the [`SessionStore`](crate::store) async operations (in-memory or Postgres),
//! then fan out via the [`Fanout`](crate::fanout) seam (tokio broadcast today,
//! Redis across instances).

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
    if state.store.ensure(&session_id, &host_id).await.is_err() {
        return; // backend unavailable
    }
    let mut rx = state.fanout.subscribe(&session_id).await;

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
                reason: e.to_string(),
            }),
        },
        ClientMessage::SubmitAnswer {
            question_id: _,
            choice,
            elapsed_ms,
        } => match state
            .store
            .submit_answer(session_id, pid, choice, elapsed_ms)
            .await
        {
            Ok(()) => broadcast(ServerMessage::AnswerReceived {
                participant_id: pid.clone(),
            }),
            Err(e) => reply(ServerMessage::Error {
                reason: e.to_string(),
            }),
        },
        ClientMessage::PushQuestion { question } => {
            if !is_host {
                return reply(host_only());
            }
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
                    correct_choice: r.correct_choice,
                    leaderboard: r.leaderboard,
                    heatmap: r.heatmap,
                }),
                Err(e) => reply(ServerMessage::Error {
                    reason: e.to_string(),
                }),
            }
        }
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
    match state.store.push_question(session_id, &question).await {
        Ok(()) => broadcast(ServerMessage::QuestionOpened { question: public }),
        Err(e) => reply(ServerMessage::Error {
            reason: e.to_string(),
        }),
    }
}
