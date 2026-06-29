//! Covers the `OpenAiCompatible` HTTP client (`provider.rs` chat/embed) against a
//! local mock `/v1/*` server — the real-provider code path **without** a real AI,
//! so it runs deterministically in CI (no AI endpoint, no flakiness). The live
//! tests (`live_provider`, `live_rag`) prove the same path against a real model.

use axum::Json;
use axum::Router;
use axum::routing::post;
use serde_json::{Value, json};

use presto_rag::provider::{AiProvider, OpenAiCompatible};

async fn mock_embeddings(Json(_body): Json<Value>) -> Json<Value> {
    // Two inputs → two 3-dim vectors, OpenAI embeddings shape.
    Json(json!({
        "data": [
            { "embedding": [0.1, 0.2, 0.3] },
            { "embedding": [0.4, 0.5, 0.6] }
        ]
    }))
}

async fn mock_chat(Json(_body): Json<Value>) -> Json<Value> {
    Json(json!({ "choices": [ { "message": { "content": "{\"ok\":true}" } } ] }))
}

#[tokio::test]
async fn openai_compatible_embed_and_chat_over_http() {
    let app = Router::new()
        .route("/v1/embeddings", post(mock_embeddings))
        .route("/v1/chat/completions", post(mock_chat));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let provider = OpenAiCompatible::new(
        format!("http://{addr}"),
        "test-key",
        "embed-model",
        "chat-model",
    );

    // embed(): two inputs → two consistent-dimension vectors.
    let vecs = provider
        .embed(&["alpha".into(), "beta".into()])
        .await
        .expect("embed");
    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0].len(), 3);
    assert_eq!(vecs[1].len(), 3);

    // complete() and complete_json() both round-trip the mocked content.
    let out = provider.complete("system", "user").await.expect("complete");
    assert!(out.contains("ok"));
    let json_out = provider
        .complete_json("system", "user")
        .await
        .expect("complete_json");
    assert!(json_out.contains("ok"));
}

#[tokio::test]
async fn openai_compatible_surfaces_http_errors() {
    // A server that 500s on both routes exercises the error_for_status branches.
    async fn boom() -> axum::http::StatusCode {
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    }
    let app = Router::new()
        .route("/v1/embeddings", post(boom))
        .route("/v1/chat/completions", post(boom));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let provider = OpenAiCompatible::new(
        format!("http://{addr}"),
        "test-key",
        "embed-model",
        "chat-model",
    );
    assert!(provider.embed(&["x".into()]).await.is_err());
    assert!(provider.complete("s", "u").await.is_err());
}
