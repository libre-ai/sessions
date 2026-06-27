//! presto-server — thin binary entry point. Builds the app state and serves it.

use std::net::SocketAddr;
use std::sync::Arc;

use presto_server::auth::Auth;
use presto_server::registry::SessionRegistry;
use presto_server::{AppState, app};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TB-1 mints with an ephemeral keypair per boot. Production loads the key
    // from `BISCUIT_PRIVATE_KEY` (so links survive restarts); identity is
    // federated via OIDC/Keycloak in TB-4.
    let state = AppState {
        registry: SessionRegistry::new(),
        auth: Arc::new(Auth::generate()),
    };

    // Clever Cloud injects `PORT`; default to 8080 for local runs.
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("presto-server listening on {addr}");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
