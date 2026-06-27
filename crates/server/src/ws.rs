//! The WebSocket session handler: one task per connection, owning its socket and
//! a subscription to the session's broadcast channel. The connection is
//! authorized by a Biscuit join token (carrying the participant id + capability)
//! BEFORE the upgrade; client messages then mutate the authoritative
//! [`Session`](crate::session::Session) under a short synchronous lock (never
//! held across an await) and fan out via the broadcast channel.

use std::time::SystemTime;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use presto_core::protocol::{ClientMessage, ServerMessage};

use crate::AppState;
use crate::auth::Claims;
use crate::registry::{SessionHandle, SessionRegistry};

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
    let registry = state.registry.clone();
    ws.on_upgrade(move |socket| handle_socket(socket, session_id, claims, name, registry))
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
    registry: SessionRegistry,
) {
    let is_host = claims.capability.is_host();
    let host_id = if is_host {
        claims.participant_id.clone()
    } else {
        String::new()
    };
    let handle = registry.get_or_create(&session_id, &host_id);
    // Subscribe BEFORE any broadcast so we never miss our own join.
    let mut rx = handle.tx.subscribe();

    if !is_host {
        let count = handle
            .session
            .lock()
            .join(claims.participant_id.clone(), name);
        let _ = handle.tx.send(ServerMessage::Joined {
            participant_id: claims.participant_id.clone(),
            participants: count,
        });
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        for reply in apply_client_message(text.as_str(), &claims, &handle) {
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
            broadcast = rx.recv() => {
                match broadcast {
                    Ok(msg) => {
                        if socket.send(Message::Text(to_text(&msg))).await.is_err() {
                            return;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {} // slow consumer dropped some; UI tolerates
                    Err(RecvError::Closed) => return,
                }
            }
        }
    }
}

/// Apply one client message; returns messages to send back ON THIS socket only.
/// Broadcasts (to every socket) are emitted via `handle.tx` here. Synchronous:
/// the session lock is never held across an await.
fn apply_client_message(text: &str, claims: &Claims, handle: &SessionHandle) -> Vec<ServerMessage> {
    let pid = &claims.participant_id;
    let is_host = claims.capability.is_host();

    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => {
            return vec![ServerMessage::Error {
                reason: "malformed message".into(),
            }];
        }
    };

    match msg {
        ClientMessage::Join { name } => {
            let count = handle.session.lock().join(pid.clone(), name);
            let _ = handle.tx.send(ServerMessage::Joined {
                participant_id: pid.clone(),
                participants: count,
            });
            vec![]
        }
        ClientMessage::SubmitAnswer {
            question_id: _,
            choice,
            elapsed_ms,
        } => match handle.session.lock().submit_answer(pid, choice, elapsed_ms) {
            Ok(()) => {
                let _ = handle.tx.send(ServerMessage::AnswerReceived {
                    participant_id: pid.clone(),
                });
                vec![]
            }
            Err(e) => vec![ServerMessage::Error {
                reason: format!("{e:?}"),
            }],
        },
        ClientMessage::PushQuestion { question } => {
            if !is_host {
                return vec![ServerMessage::Error {
                    reason: "host only".into(),
                }];
            }
            let public = question.public();
            handle.session.lock().push_question(question);
            let _ = handle
                .tx
                .send(ServerMessage::QuestionOpened { question: public });
            vec![]
        }
        ClientMessage::Reveal => {
            if !is_host {
                return vec![ServerMessage::Error {
                    reason: "host only".into(),
                }];
            }
            match handle.session.lock().reveal() {
                Ok(r) => {
                    let _ = handle.tx.send(ServerMessage::AnswersRevealed {
                        correct_choice: r.correct_choice,
                        leaderboard: r.leaderboard,
                        heatmap: r.heatmap,
                    });
                    vec![]
                }
                Err(e) => vec![ServerMessage::Error {
                    reason: format!("{e:?}"),
                }],
            }
        }
        ClientMessage::Ping => vec![ServerMessage::Pong],
    }
}
