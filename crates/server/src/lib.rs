//! presto-server — the Presto-Matic backend library.
//!
//! The authoritative live-session engine ([`session`]), the in-memory registry +
//! fanout ([`registry`]), the Biscuit join-link authorization ([`auth`]), and the
//! WebSocket handler ([`ws`]) live here as testable library code; `src/main.rs`
//! is the thin binary entry point. Later slices add the distributed
//! (Redis/Postgres) seams and OIDC/Keycloak identity federation.

pub mod auth;
pub mod registry;
pub mod session;
pub mod ws;

use std::sync::Arc;

use axum::{Router, routing::get};

use auth::Auth;
use registry::SessionRegistry;

/// Shared application state: the live-session registry and the token authority.
#[derive(Clone)]
pub struct AppState {
    pub registry: SessionRegistry,
    pub auth: Arc<Auth>,
}

/// Build the application router over shared state.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws/{session_id}", get(ws::ws_handler))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> AppState {
        AppState {
            registry: SessionRegistry::new(),
            auth: Arc::new(Auth::generate()),
        }
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let response = app(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
