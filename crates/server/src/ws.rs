//! The WebSocket session handler: one task per connection. The connection is
//! authorized by a Biscuit join token BEFORE the upgrade. Client messages mutate
//! authoritative session state (from the [`SessionStore`](crate::store)) under a
//! short synchronous lock — never across an await — then fan out via the
//! [`Fanout`](crate::fanout) seam (tokio broadcast today, Redis across instances).

use std::time::SystemTime;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use parking_lot::Mutex;
use serde::Deserialize;

use presto_core::protocol::{ClientMessage, ServerMessage};

use crate::AppState;
use crate::auth::Claims;
use crate::session::Session;

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
    let session = state.store.get_or_create(&session_id, &host_id);
    let mut rx = state.fanout.subscribe(&session_id).await;

    if !is_host {
        let count = session.lock().join(claims.participant_id.clone(), name);
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

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let applied = apply(text.as_str(), &claims, &session);
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

/// Mutates the session under a short lock and returns what to send. No I/O here:
/// the caller fans out broadcasts and writes replies (the lock is never held
/// across an await).
fn apply(text: &str, claims: &Claims, session: &Mutex<Session>) -> Applied {
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
        ClientMessage::Join { name } => {
            let count = session.lock().join(pid.clone(), name);
            broadcast(ServerMessage::Joined {
                participant_id: pid.clone(),
                participants: count,
            })
        }
        ClientMessage::SubmitAnswer {
            question_id: _,
            choice,
            elapsed_ms,
        } => match session.lock().submit_answer(pid, choice, elapsed_ms) {
            Ok(()) => broadcast(ServerMessage::AnswerReceived {
                participant_id: pid.clone(),
            }),
            Err(e) => reply(ServerMessage::Error {
                reason: format!("{e:?}"),
            }),
        },
        ClientMessage::PushQuestion { question } => {
            if !is_host {
                return reply(ServerMessage::Error {
                    reason: "host only".into(),
                });
            }
            let public = question.public();
            session.lock().push_question(question);
            broadcast(ServerMessage::QuestionOpened { question: public })
        }
        ClientMessage::Reveal => {
            if !is_host {
                return reply(ServerMessage::Error {
                    reason: "host only".into(),
                });
            }
            match session.lock().reveal() {
                Ok(r) => broadcast(ServerMessage::AnswersRevealed {
                    correct_choice: r.correct_choice,
                    leaderboard: r.leaderboard,
                    heatmap: r.heatmap,
                }),
                Err(e) => reply(ServerMessage::Error {
                    reason: format!("{e:?}"),
                }),
            }
        }
        ClientMessage::Ping => reply(ServerMessage::Pong),
    }
}
