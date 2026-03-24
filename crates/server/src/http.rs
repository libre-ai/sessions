//! HTTP surface beyond the live WebSocket: session creation and participant join
//! (which mint Biscuit tokens), plus the static web client.
//!
//! `POST /sessions` is open (anyone can host) for the wedge; real host identity
//! (OIDC/Keycloak) sits in front later. The token — not the session code — is the
//! capability, so a short, human-typable code is fine.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use serde::Serialize;

use crate::AppState;
use crate::auth::Capability;

const TOKEN_TTL: Duration = Duration::from_secs(6 * 3600);
/// Unambiguous alphabet (no 0/O/1/I) for human-typable codes.
const CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

fn code(n: usize) -> String {
    (0..n)
        .map(|_| CODE_CHARS[rand::random_range(0..CODE_CHARS.len())] as char)
        .collect()
}

#[derive(Serialize)]
pub(crate) struct Envelope<T> {
    data: T,
}

#[derive(Serialize)]
pub(crate) struct CreatedSession {
    session_id: String,
    host_token: String,
    join_url: String,
}

/// Create a session and return a host token + a participant join URL.
pub(crate) async fn create_session(
    State(state): State<AppState>,
) -> Result<Json<Envelope<CreatedSession>>, StatusCode> {
    let session_id = code(6);
    let host_id = format!("host-{}", code(4));
    state
        .store
        .ensure(&session_id, &host_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let host_token = state
        .auth
        .mint(&session_id, &host_id, Capability::Host, TOKEN_TTL)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let join_url = format!("/?s={session_id}");
    Ok(Json(Envelope {
        data: CreatedSession {
            session_id,
            host_token,
            join_url,
        },
    }))
}

#[derive(Serialize)]
pub(crate) struct JoinedSession {
    participant_id: String,
    participant_token: String,
}

/// Mint a participant token for a session. The display name travels on the WS
/// connect (`?name=`), so no request body is needed here.
pub(crate) async fn join_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Envelope<JoinedSession>>, StatusCode> {
    // Only mint a token for a real session (no tokens for arbitrary ids).
    if !state
        .store
        .exists(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let participant_id = format!("p-{}", code(6));
    let participant_token = state
        .auth
        .mint(
            &session_id,
            &participant_id,
            Capability::Participant,
            TOKEN_TTL,
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(Envelope {
        data: JoinedSession {
            participant_id,
            participant_token,
        },
    }))
}

pub(crate) async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

pub(crate) async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/app.js"),
    )
}
