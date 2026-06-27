//! presto-server — the Presto-Matic backend library.
//!
//! The authoritative live-session engine ([`session`]), the seams for state and
//! fanout ([`store`], [`fanout`]), the Biscuit join-link authorization ([`auth`]),
//! and the WebSocket handler ([`ws`]) live here as testable library code;
//! `src/main.rs` is the thin binary entry point. The [`store::SessionStore`] and
//! [`fanout::Fanout`] traits are the seams where the distributed (Redis /
//! Postgres) implementations plug in for multi-instance operation.

pub mod auth;
pub mod fanout;
pub mod postgres_store;
pub mod redis_fanout;
pub mod session;
pub mod store;
pub mod ws;

use std::sync::Arc;

use axum::{Router, routing::get};

use auth::Auth;
use fanout::{BroadcastFanout, Fanout};
use store::{InMemorySessionStore, SessionStore};

/// Shared application state: the session-state store, the fanout, and the token
/// authority. The store/fanout are trait objects so a deployment chooses
/// single-instance (in-memory) or multi-instance (Redis/Postgres) at startup.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn SessionStore>,
    pub fanout: Arc<dyn Fanout>,
    pub auth: Arc<Auth>,
}

impl AppState {
    /// Single-instance state: in-memory store + tokio-broadcast fanout.
    pub fn in_memory(auth: Arc<Auth>) -> Self {
        Self {
            store: Arc::new(InMemorySessionStore::new()),
            fanout: Arc::new(BroadcastFanout::new()),
            auth,
        }
    }
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

    #[tokio::test]
    async fn health_returns_ok() {
        let state = AppState::in_memory(Arc::new(Auth::generate()));
        let response = app(state)
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
