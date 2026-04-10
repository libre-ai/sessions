//! HTTP surface beyond the live WebSocket: session creation and participant join
//! (which mint Biscuit tokens), plus the static web client.
//!
//! `POST /sessions` is open (anyone can host) for the wedge; real host identity
//! (OIDC/Keycloak) sits in front later. The token — not the session code — is the
//! capability, so a short, human-typable code is fine.

use std::time::{Duration, SystemTime};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use serde::{Deserialize, Serialize};

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
        .mint(
            &session_id,
            &host_id,
            Capability::Host,
            TOKEN_TTL,
            SystemTime::now(),
        )
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
            SystemTime::now(),
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(Envelope {
        data: JoinedSession {
            participant_id,
            participant_token,
        },
    }))
}

#[derive(Deserialize)]
pub(crate) struct IngestParams {
    document_id: String,
}

#[derive(Serialize)]
pub(crate) struct IngestResult {
    document_id: String,
    chunks_stored: usize,
}

/// Constant-time byte comparison for the ingest token (avoids leaking how many
/// leading bytes matched via response timing).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Ingest a `text/plain` or `text/markdown` document into the corpus (parse →
/// chunk → embed → store) so the RAG sources ground on it. Optionally gated by a
/// bearer `INGEST_TOKEN` (open when the env var is unset, for local dev).
pub(crate) async fn ingest_document(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<IngestParams>,
    body: Bytes,
) -> Result<Json<Envelope<IngestResult>>, (StatusCode, String)> {
    if let Ok(expected) = std::env::var("INGEST_TOKEN") {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .unwrap_or("");
        if !ct_eq(presented.as_bytes(), expected.as_bytes()) {
            return Err((StatusCode::UNAUTHORIZED, "invalid ingest token".into()));
        }
    }
    if params.document_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "document_id is required".into()));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let chunks_stored = state
        .ingestor
        .ingest(&params.document_id, content_type, &body)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(Envelope {
        data: IngestResult {
            document_id: params.document_id,
            chunks_stored,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_matches_only_identical_bytes() {
        assert!(ct_eq(b"secret", b"secret"));
        assert!(ct_eq(b"", b""));
        assert!(!ct_eq(b"secret", b"secrXt")); // same length, one byte differs
        assert!(!ct_eq(b"secret", b"secre")); // different length
        assert!(!ct_eq(b"", b"x"));
    }
}
