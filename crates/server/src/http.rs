//! HTTP surface beyond the live WebSocket: session creation and participant join
//! (which mint Biscuit tokens), plus the static web client.
//!
//! `POST /sessions` is open (anyone can host) for the wedge; real host identity
//! (OIDC/Keycloak) sits in front later. The token — not the session code — is the
//! capability, so a short, human-typable code is fine.

use std::time::{Duration, SystemTime};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use serde::{Deserialize, Serialize};

use presto_core::WorkspaceIdentity;
use presto_core::p0_contract::{
    P0StubWorkflowProof, P0ValidationReport, run_p0_stub_workflow, valid_p0_fixture,
    validate_p0_fixture,
};
use presto_core::protocol::{
    MAX_SESSION_PARTICIPANT_NAME_BYTES, MAX_SESSION_PARTICIPANT_NAME_CHARS,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::auth::Capability;
use crate::quiz::IngestRejection;
use crate::session_identity::{SessionRole, SessionScope, workspace_identity_for_actor};

const TOKEN_TTL: Duration = Duration::from_secs(6 * 3600);
const JOIN_LINK_TTL: Duration = Duration::from_secs(30 * 60);
/// Unambiguous alphabet (no 0/O/1/I) for human-typable codes.
const CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const LEGACY_SESSION_CODE_LEN: usize = 6;
const CURRENT_SESSION_CODE_LEN: usize = 12;
pub(crate) const MAX_LEGACY_INGEST_BODY_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CONCURRENT_LEGACY_INGESTS: usize = 4;
pub(crate) const MAX_JOIN_REDEMPTION_BODY_BYTES: usize = 256;
pub(crate) const MAX_CONCURRENT_JOIN_REDEMPTIONS: usize = 8;

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
    secure_join_url: String,
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
    let session_id = code(CURRENT_SESSION_CODE_LEN);
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
    let join_token = state
        .auth
        .mint_join_link(&scope, JOIN_LINK_TTL, SystemTime::now())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let workspace_identity = workspace_identity_for_actor(&scope, &host_id, SessionRole::Host);
    let join_url = format!("/?s={session_id}");
    let secure_join_url = format!("/join/{session_id}#token={join_token}");
    Ok(Json(Envelope {
        data: CreatedSession {
            tenant_id: scope.tenant_id,
            workspace_id: scope.workspace_id,
            session_id,
            host_token,
            join_url,
            secure_join_url,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
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
    let participant_id = format!("p-{}", uuid::Uuid::new_v4().simple());
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
            name: None,
            workspace_identity,
        },
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JoinParticipantRequest {
    name: String,
}

fn join_path_session_id(path: &str) -> Option<&str> {
    path.strip_prefix("/join/")
        .and_then(|rest| rest.strip_suffix("/participants"))
        .filter(|session_id| validate_session_code(session_id))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
}

fn validate_session_code(session_id: &str) -> bool {
    matches!(
        session_id.len(),
        LEGACY_SESSION_CODE_LEN | CURRENT_SESSION_CODE_LEN
    ) && session_id.bytes().all(|byte| CODE_CHARS.contains(&byte))
}

pub(crate) fn validate_join_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_SESSION_PARTICIPANT_NAME_CHARS
        || trimmed.len() > MAX_SESSION_PARTICIPANT_NAME_BYTES
        || trimmed.chars().any(|ch| ch.is_control())
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(crate) async fn authorize_join_redemption(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = bearer_token(request.headers()) else {
        return join_unavailable(StatusCode::UNAUTHORIZED);
    };
    let Some(session_id) = join_path_session_id(request.uri().path()) else {
        return join_unavailable(StatusCode::UNAUTHORIZED);
    };
    if !state.join_redemption_rate.allow() {
        return join_unavailable(StatusCode::TOO_MANY_REQUESTS);
    }
    let scope = SessionScope::for_session(session_id);
    if state
        .auth
        .verify_join_link(token, &scope, SystemTime::now())
        .is_err()
    {
        return join_unavailable(StatusCode::UNAUTHORIZED);
    }
    match state.store.exists(session_id).await {
        Ok(true) => {}
        Ok(false) => return join_unavailable(StatusCode::UNAUTHORIZED),
        Err(_) => return join_unavailable(StatusCode::SERVICE_UNAVAILABLE),
    }
    next.run(request).await
}

fn join_unavailable(status: StatusCode) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        "join unavailable",
    )
        .into_response()
}

pub(crate) async fn redeem_join_link(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<JoinParticipantRequest>,
) -> Result<Json<Envelope<JoinedSession>>, (StatusCode, String)> {
    if !validate_session_code(&session_id) {
        return Err((StatusCode::UNAUTHORIZED, "join unavailable".into()));
    }
    let name = validate_join_name(&payload.name)
        .ok_or((StatusCode::BAD_REQUEST, "invalid name".into()))?;
    let scope = SessionScope::for_session(&session_id);
    let exists = state
        .store
        .exists(&session_id)
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "join unavailable".into()))?;
    if !exists {
        return Err((StatusCode::UNAUTHORIZED, "join unavailable".into()));
    }
    let participant_id = format!("p-{}", uuid::Uuid::new_v4().simple());
    state
        .store
        .join(&session_id, &participant_id, &name)
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "join unavailable".into()))?;
    let participant_token = state
        .auth
        .mint_scoped(
            &scope,
            &participant_id,
            Capability::Participant,
            TOKEN_TTL,
            SystemTime::now(),
        )
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "join unavailable".into()))?;
    let workspace_identity =
        workspace_identity_for_actor(&scope, &participant_id, SessionRole::Participant);
    Ok(Json(Envelope {
        data: JoinedSession {
            tenant_id: scope.tenant_id,
            workspace_id: scope.workspace_id,
            participant_id,
            participant_token,
            name: Some(name),
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

/// Validate configuration once at the composition root. Tokens are deliberately
/// strong and header-safe; absence or weak values are configuration errors.
pub fn validate_legacy_ingest_token(value: &str) -> bool {
    (32..=512).contains(&value.len()) && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// Fail-closed legacy ingestion gate. Digest comparison has fixed length and is
/// performed before the request body limit/extractor and expensive ingestion.
pub(crate) async fn authorize_legacy_ingest(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let expected = state
        .legacy_ingest_token
        .as_deref()
        .filter(|token| validate_legacy_ingest_token(token));
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    let accepted = expected
        .map(|expected| {
            let expected_digest = Sha256::digest(expected.as_bytes());
            let presented_digest = Sha256::digest(presented.as_bytes());
            bool::from(expected_digest.ct_eq(&presented_digest))
        })
        .unwrap_or(false);
    if !accepted {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::CACHE_CONTROL, "no-store")],
            "invalid ingest token",
        )
            .into_response();
    }
    next.run(request).await
}

/// Ingest a `text/plain` or `text/markdown` document into the corpus (parse →
/// chunk → embed → store) so the RAG sources ground on it. Authentication has
/// already completed in the pre-body route middleware.
pub(crate) async fn ingest_document(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<IngestParams>,
    body: Bytes,
) -> Result<Json<Envelope<IngestResult>>, (StatusCode, String)> {
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

/// Dioxus owner shell entry point. Nested `/app/*` browser routes return this
/// same document; the client router selects the screen after WASM starts.
pub(crate) const OWNER_APP_CSP: &str = "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; style-src-attr 'none'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'";
const OWNER_PERMISSIONS_POLICY: &str =
    "accelerometer=(), camera=(), geolocation=(), gyroscope=(), microphone=(), payment=(), usb=()";

fn secure_owner_response(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(OWNER_APP_CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static(OWNER_PERMISSIONS_POLICY),
    );
    response
}

pub(crate) async fn owner_app_index() -> Response {
    let mut response = Html(include_str!("../static/owner-app/index.html")).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    secure_owner_response(response)
}

pub(crate) struct EmbeddedOwnerFile {
    pub(crate) path: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) etag: &'static str,
    pub(crate) body: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/owner_app_assets.rs"));

fn owner_file(path: &str, request_headers: &HeaderMap) -> Response {
    let Some(found) = OWNER_APP_FILES.iter().find(|item| item.path == path) else {
        return secure_owner_response(StatusCode::NOT_FOUND.into_response());
    };
    let etag = format!("\"{}\"", found.etag);
    let not_modified = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str());
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (StatusCode::OK, found.body).into_response()
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(found.content_type),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("valid ETag"),
    );
    let cache_control = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    if path == "sw.js" {
        headers.insert("service-worker-allowed", HeaderValue::from_static("/app/"));
    }
    secure_owner_response(response)
}

pub(crate) async fn owner_app_asset(Path(asset): Path<String>, headers: HeaderMap) -> Response {
    owner_file(&format!("assets/{asset}"), &headers)
}

pub(crate) async fn owner_app_icon(Path(icon): Path<String>, headers: HeaderMap) -> Response {
    owner_file(&format!("icons/{icon}"), &headers)
}

pub(crate) async fn owner_app_manifest(headers: HeaderMap) -> Response {
    owner_file("manifest.webmanifest", &headers)
}

pub(crate) async fn owner_app_internal_manifest(headers: HeaderMap) -> Response {
    owner_file("owner-shell-manifest.json", &headers)
}

pub(crate) async fn owner_app_service_worker(headers: HeaderMap) -> Response {
    owner_file("sw.js", &headers)
}

pub(crate) struct EmbeddedJoinFile {
    pub(crate) path: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) etag: &'static str,
    pub(crate) body: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/join_app_assets.rs"));

const JOIN_APP_CSP: &str = "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; style-src-attr 'none'; img-src 'self'; font-src 'self'; connect-src 'self'; worker-src 'none'; manifest-src 'none'";

fn secure_join_response(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(JOIN_APP_CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static(OWNER_PERMISSIONS_POLICY),
    );
    response
}

pub(crate) async fn join_app_index() -> Response {
    let mut response = Html(include_str!("../static/join-app/index.html")).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    secure_join_response(response)
}

fn join_file(path: &str, request_headers: &HeaderMap) -> Response {
    let Some(found) = JOIN_APP_FILES.iter().find(|item| item.path == path) else {
        return secure_join_response(StatusCode::NOT_FOUND.into_response());
    };
    let etag = format!("\"{}\"", found.etag);
    let not_modified = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str());
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (StatusCode::OK, found.body).into_response()
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(found.content_type),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("valid ETag"),
    );
    let cache_control = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    secure_join_response(response)
}

pub(crate) async fn join_app_asset(Path(asset): Path<String>, headers: HeaderMap) -> Response {
    join_file(&format!("assets/{asset}"), &headers)
}

pub(crate) async fn join_app_internal_manifest(headers: HeaderMap) -> Response {
    join_file("join-shell-manifest.json", &headers)
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
    fn join_boundary_rejects_ambiguous_bearers_and_invalid_codes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer token-one"),
        );
        assert_eq!(bearer_token(&headers), Some("token-one"));
        headers.append(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer token-two"),
        );
        assert_eq!(bearer_token(&headers), None);

        assert!(validate_session_code("ABCDEF"));
        assert!(validate_session_code("ABCDEFGHJKLM"));
        assert!(!validate_session_code("ABCDE"));
        assert!(!validate_session_code("ABCDEFG"));
        assert!(!validate_session_code("ABCDEFGHJKLMN"));
        assert!(!validate_session_code("ABC0EF"));
    }

    #[test]
    fn legacy_ingest_token_requires_strong_header_safe_entropy() {
        assert!(validate_legacy_ingest_token(
            "0123456789abcdef0123456789abcdef"
        ));
        assert!(validate_legacy_ingest_token(&"~".repeat(512)));
        assert!(!validate_legacy_ingest_token(""));
        assert!(!validate_legacy_ingest_token("short"));
        assert!(!validate_legacy_ingest_token(
            "0123456789abcdef 0123456789abcdef"
        ));
        assert!(!validate_legacy_ingest_token(
            "0123456789abcdefé0123456789abcdef"
        ));
        assert!(!validate_legacy_ingest_token(
            "0123456789abcdef\u{80}0123456789abcdef"
        ));
        assert!(!validate_legacy_ingest_token(
            "0123456789abcdef\u{7f}0123456789abcdef"
        ));
        assert!(!validate_legacy_ingest_token(&"x".repeat(513)));
    }
}
