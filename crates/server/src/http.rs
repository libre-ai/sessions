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

use presto_core::WorkspaceIdentity;
use presto_core::p0_contract::{
    P0StubWorkflowProof, P0ValidationReport, run_p0_stub_workflow, valid_p0_fixture,
    validate_p0_fixture,
};

use crate::AppState;
use crate::auth::Capability;
use crate::quiz::IngestRejection;
use crate::session_identity::{SessionRole, SessionScope, workspace_identity_for_actor};

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
    tenant_id: String,
    workspace_id: String,
    session_id: String,
    host_token: String,
    join_url: String,
    workspace_identity: WorkspaceIdentity,
}

/// Create a session and return a host token + a participant join URL.
pub(crate) async fn create_session(
    State(state): State<AppState>,
) -> Result<Json<Envelope<CreatedSession>>, StatusCode> {
    // The endpoint is open (wedge), so rate-limit creation to bound resource use.
    if !state.session_rate.allow() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let session_id = code(6);
    let scope = SessionScope::for_session(&session_id);
    let host_id = format!("host-{}", code(4));
    state
        .store
        .ensure(&session_id, &host_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let host_token = state
        .auth
        .mint_scoped(
            &scope,
            &host_id,
            Capability::Host,
            TOKEN_TTL,
            SystemTime::now(),
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let workspace_identity = workspace_identity_for_actor(&scope, &host_id, SessionRole::Host);
    let join_url = format!("/?s={session_id}");
    Ok(Json(Envelope {
        data: CreatedSession {
            tenant_id: scope.tenant_id,
            workspace_id: scope.workspace_id,
            session_id,
            host_token,
            join_url,
            workspace_identity,
        },
    }))
}

#[derive(Serialize)]
pub(crate) struct JoinedSession {
    tenant_id: String,
    workspace_id: String,
    participant_id: String,
    participant_token: String,
    workspace_identity: WorkspaceIdentity,
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
    let scope = SessionScope::for_session(&session_id);
    let participant_id = format!("p-{}", code(6));
    let participant_token = state
        .auth
        .mint_scoped(
            &scope,
            &participant_id,
            Capability::Participant,
            TOKEN_TTL,
            SystemTime::now(),
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let workspace_identity =
        workspace_identity_for_actor(&scope, &participant_id, SessionRole::Participant);
    Ok(Json(Envelope {
        data: JoinedSession {
            tenant_id: scope.tenant_id,
            workspace_id: scope.workspace_id,
            participant_id,
            participant_token,
            workspace_identity,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct P0ContractProof {
    scope: &'static str,
    report: P0ValidationReport,
    boundaries_proved: Vec<&'static str>,
    execution: P0ContractExecution,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct P0ContractExecution {
    ui_executed: bool,
    wrench_called: bool,
    gear_called: bool,
    bolt_called: bool,
    biscuit_runtime_called: bool,
    llm_provider_called: bool,
}

/// Contract-only P0 proof endpoint. It validates the core fixture and proves the
/// server can expose the source-grounded boundary without calling UI, providers,
/// Wrench, Gear, Bolt, or Biscuit runtimes.
pub(crate) async fn p0_contract_proof() -> Json<Envelope<P0ContractProof>> {
    let report = validate_p0_fixture(&valid_p0_fixture());
    Json(Envelope {
        data: P0ContractProof {
            scope: "fixture-only contract proof; no product runtime or external provider called",
            report,
            boundaries_proved: vec![
                "Rumble LM stores source-set refs/snapshots, not source truth.",
                "Wrench/Gear-shaped source provenance is required before grounding.",
                "Bolt-shaped generation remains draft-only and cannot publish.",
                "Validated citations are required for source-derived generated claims.",
                "Participant-facing exports exclude private responses by default.",
                "Delegations are scoped, expiring, revocable, and least-privilege.",
                "Default analytics are aggregate-only with no hidden learner profile.",
                "Sovereignty filters block mandatory US SaaS, opaque storage, blocking licenses, silent provider fallback, and PII logs.",
            ],
            execution: P0ContractExecution {
                ui_executed: false,
                wrench_called: false,
                gear_called: false,
                bolt_called: false,
                biscuit_runtime_called: false,
                llm_provider_called: false,
            },
        },
    })
}

/// Run the deterministic P0 vertical stub. This remains contract-only: no
/// persistence, external providers, Wrench/Gear/Bolt calls, or Biscuit runtime.
pub(crate) async fn p0_stub_run() -> Json<Envelope<P0StubWorkflowProof>> {
    Json(Envelope {
        data: run_p0_stub_workflow(),
    })
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
    if params.document_id.len() > 128 {
        return Err((
            StatusCode::BAD_REQUEST,
            "document_id too long (max 128 bytes)".into(),
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let chunks_stored = state
        .ingestor
        .ingest(&params.document_id, content_type, &body)
        .await
        .map_err(|r| match r {
            // Only a document fault is described to the client; backend detail is
            // logged in the ingestor, not surfaced.
            IngestRejection::BadDocument(msg) => (StatusCode::BAD_REQUEST, msg),
            IngestRejection::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "document ingestion is not configured".into(),
            ),
            IngestRejection::Backend => {
                (StatusCode::INTERNAL_SERVER_ERROR, "ingestion failed".into())
            }
        })?;
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
