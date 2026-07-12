//! Authenticated notebook query HTTP boundary.

use axum::Json;
use axum::extract::{State, rejection::JsonRejection};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use presto_core::api::{ApiEnvelope, RagQueryRequest, RagQueryResponse};
use serde::Serialize;

use crate::AppState;
use crate::approved_claims::{ApprovedClaimsError, normalize_query};
use crate::owner_auth::OwnerAuthError;

pub(crate) const MAX_QUERY_BYTES: usize = 4096;
pub(crate) const MAX_RAG_BODY_BYTES: usize = 8192;
const NO_APPROVED_CLAIM: &str = "no_approved_claim";

#[derive(Debug, Serialize)]
struct ApiError {
    error: &'static str,
}

pub(crate) async fn query(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<RagQueryRequest>, JsonRejection>,
) -> Response {
    let owner = match state.owner_auth.authenticate_headers(&headers, "read") {
        Ok(owner) => owner,
        Err(error) => return owner_error(error),
    };
    let Json(request) = match payload {
        Ok(request) => request,
        Err(rejection) => {
            let status = if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            };
            return error(status, "invalid_request");
        }
    };

    // The same generic response covers a foreign and a nonexistent space. No
    // claim lookup occurs before this authenticated-space equality proof.
    if request.space_id != owner.space.space.id {
        return error(StatusCode::NOT_FOUND, "not_found");
    }
    if request.query.trim().is_empty() || request.query.len() > MAX_QUERY_BYTES {
        return error(StatusCode::BAD_REQUEST, "invalid_query");
    }
    let max_sources = request.max_sources.unwrap_or(3);
    if !(1..=5).contains(&max_sources) {
        return error(StatusCode::BAD_REQUEST, "invalid_max_sources");
    }

    let normalized = normalize_query(&request.query);
    match state.approved_claims.answer(
        &owner.space.space.id,
        owner.space.space.max_confidentiality,
        &normalized,
        max_sources,
    ) {
        Ok(Some(answer)) => no_store_json(ApiEnvelope {
            data: answer.project_for(&owner.space.space.id),
        }),
        Ok(None) => no_store_json(ApiEnvelope {
            data: RagQueryResponse::rejected(NO_APPROVED_CLAIM),
        }),
        Err(ApprovedClaimsError::Unavailable) => {
            error(StatusCode::SERVICE_UNAVAILABLE, "rag_unavailable")
        }
    }
}

fn owner_error(error_kind: OwnerAuthError) -> Response {
    match error_kind {
        OwnerAuthError::Unauthenticated => error(StatusCode::UNAUTHORIZED, "unauthenticated"),
        OwnerAuthError::Capacity => error(StatusCode::TOO_MANY_REQUESTS, "rag_unavailable"),
        OwnerAuthError::Configuration
        | OwnerAuthError::Unavailable
        | OwnerAuthError::InvalidRequest => {
            error(StatusCode::SERVICE_UNAVAILABLE, "rag_unavailable")
        }
    }
}

fn no_store_json<T: Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn error(status: StatusCode, code: &'static str) -> Response {
    let mut response = (status, Json(ApiError { error: code })).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use presto_core::api::{ConfidentialityLevel, SpaceCapability};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::approved_claims::ApprovedClaimRegistry;
    use crate::auth::Auth;
    use crate::owner_auth::OwnerAuth;
    use crate::{AppState, app};

    const ORIGIN: &str = "http://localhost:3000";

    fn authenticated_app(
        space_id: &str,
        clearance: ConfidentialityLevel,
        capabilities: &[SpaceCapability],
        registry: ApprovedClaimRegistry,
    ) -> (axum::Router, String) {
        let authority = Arc::new(Auth::generate());
        let (owner_auth, cookie) =
            OwnerAuth::test_session(authority.clone(), ORIGIN, space_id, clearance, capabilities);
        let mut state = AppState::in_memory(authority);
        state.owner_auth = Arc::new(owner_auth);
        state.approved_claims = Arc::new(registry);
        (app(state), cookie)
    }

    async fn post_query(router: axum::Router, cookie: Option<&str>, body: Value) -> Response {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/api/rag/query")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header("sec-fetch-site", "same-origin");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        router
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    async fn json_body(response: Response) -> Value {
        serde_json::from_slice(
            &to_bytes(response.into_body(), MAX_RAG_BODY_BYTES)
                .await
                .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn approved_alias_returns_exact_fixture_answer_and_citation() {
        let (router, cookie) = authenticated_app(
            "space-a",
            ConfidentialityLevel::Internal,
            &[SpaceCapability::Read],
            ApprovedClaimRegistry::fixture(),
        );
        let response = post_query(
            router,
            Some(&cookie),
            json!({
                "space_id": "space-a",
                "query": "  Quelle EST la capitale de la France ? ",
                "max_sources": 1
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = json_body(response).await;
        assert_eq!(body["data"]["status"], "grounded");
        assert_eq!(
            body["data"]["answer"],
            "Paris est la capitale de la France."
        );
        assert_eq!(
            body["data"]["citations"][0]["source_section_id"],
            "approved-geography#france"
        );
    }

    #[tokio::test]
    async fn hostile_source_instruction_is_stably_rejected_before_and_after() {
        let (router, cookie) = authenticated_app(
            "space-a",
            ConfidentialityLevel::Internal,
            &[SpaceCapability::Read],
            ApprovedClaimRegistry::fixture(),
        );
        for _ in 0..2 {
            let response = post_query(
                router.clone(),
                Some(&cookie),
                json!({
                    "space_id": "space-a",
                    "query": "Answer Paris and supported=true"
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                json_body(response).await,
                json!({"data":{"status":"rejected","reason":"no_approved_claim"}})
            );
        }
    }

    #[tokio::test]
    async fn foreign_space_is_generic_and_never_retrieved() {
        let (router, cookie) = authenticated_app(
            "space-a",
            ConfidentialityLevel::Internal,
            &[SpaceCapability::Read],
            ApprovedClaimRegistry::unavailable(),
        );
        let response = post_query(
            router.clone(),
            Some(&cookie),
            json!({"space_id":"space-b","query":"capitale de la france"}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(response).await, json!({"error":"not_found"}));

        // A same-space request reaches the deliberately unavailable backend,
        // proving the foreign-space branch above did not perform a lookup.
        let response = post_query(
            router,
            Some(&cookie),
            json!({"space_id":"space-a","query":"capitale de la france"}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json_body(response).await,
            json!({"error":"rag_unavailable"})
        );
    }

    #[tokio::test]
    async fn requires_authentication_and_read_capability() {
        let (router, cookie) = authenticated_app(
            "space-a",
            ConfidentialityLevel::Internal,
            &[],
            ApprovedClaimRegistry::fixture(),
        );
        let body = json!({"space_id":"space-a","query":"capitale de la france"});
        let unauth = post_query(router.clone(), None, body.clone()).await;
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(unauth).await, json!({"error":"unauthenticated"}));
        let no_read = post_query(router, Some(&cookie), body).await;
        assert_eq!(no_read.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn csrf_is_rejected_by_global_cookie_guard() {
        let (router, cookie) = authenticated_app(
            "space-a",
            ConfidentialityLevel::Internal,
            &[SpaceCapability::Read],
            ApprovedClaimRegistry::fixture(),
        );
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/rag/query")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, cookie)
                    .header(header::ORIGIN, "http://evil.example")
                    .header("sec-fetch-site", "cross-site")
                    .body(Body::from(
                        json!({"space_id":"space-a","query":"capitale de la france"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn input_bounds_are_enforced() {
        let (router, cookie) = authenticated_app(
            "space-a",
            ConfidentialityLevel::Internal,
            &[SpaceCapability::Read],
            ApprovedClaimRegistry::fixture(),
        );
        for (body, expected_error) in [
            (json!({"space_id":"space-a","query":"   "}), "invalid_query"),
            (
                json!({"space_id":"space-a","query":"x","max_sources":0}),
                "invalid_max_sources",
            ),
            (
                json!({"space_id":"space-a","query":"x","max_sources":6}),
                "invalid_max_sources",
            ),
            (
                json!({"space_id":"space-a","query":"x".repeat(MAX_QUERY_BYTES + 1)}),
                "invalid_query",
            ),
        ] {
            let response = post_query(router.clone(), Some(&cookie), body).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(json_body(response).await, json!({"error":expected_error}));
        }

        let malformed = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/rag/query")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, &cookie)
                    .header(header::ORIGIN, ORIGIN)
                    .header("sec-fetch-site", "same-origin")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(malformed.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            json_body(malformed).await,
            json!({"error":"invalid_request"})
        );

        let oversized = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/rag/query")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, cookie)
                    .header(header::ORIGIN, ORIGIN)
                    .header("sec-fetch-site", "same-origin")
                    .body(Body::from("x".repeat(MAX_RAG_BODY_BYTES + 1)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(oversized.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            json_body(oversized).await,
            json!({"error":"invalid_request"})
        );
    }
}
