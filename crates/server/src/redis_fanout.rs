//! Multi-instance fanout via Redis pub/sub. Each instance keeps ONE Redis
//! subscriber per live session, re-broadcasting received messages onto a local
//! tokio broadcast (so connections share one Redis subscription). Publishing
//! goes to Redis, which delivers to every instance — including this one, so no
//! separate local echo is needed.
//!
//! NOTE: session STATE is still per-instance here; correct cross-instance
//! scoring needs the Postgres `SessionStore` (next slice). This module proves
//! the *message* fanout crosses instances.

use std::collections::HashMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use parking_lot::Mutex;
use redis::AsyncCommands;
use redis::aio::MultiplexedConnection;
use tokio::sync::{broadcast, mpsc};

use presto_core::protocol::ServerMessage;

use crate::fanout::{BUFFER, Fanout};

fn channel_name(session_id: &str) -> String {
    format!("presto:session:{session_id}")
}

/// Redis-backed [`Fanout`] for multi-instance operation.
pub struct RedisFanout {
    client: redis::Client,
    publisher: MultiplexedConnection,
    locals: Mutex<HashMap<String, broadcast::Sender<ServerMessage>>>,
}

impl RedisFanout {
    /// Connect to Redis at `url` (e.g. `redis://127.0.0.1/`).
    pub async fn connect(url: &str) -> redis::RedisResult<Self> {
        let client = redis::Client::open(url)?;
        let publisher = client.get_multiplexed_async_connection().await?;
        Ok(Self {
            client,
            publisher,
            locals: Mutex::new(HashMap::new()),
        })
    }

    /// The per-session local broadcast, creating it (and its Redis subscriber
    /// task) on first use.
    fn local(&self, session_id: &str) -> broadcast::Sender<ServerMessage> {
        let mut map = self.locals.lock();
        if let Some(tx) = map.get(session_id) {
            return tx.clone();
        }
        let (tx, _rx) = broadcast::channel(BUFFER);
        map.insert(session_id.to_string(), tx.clone());

        let client = self.client.clone();
        let channel = channel_name(session_id);
        let sink = tx.clone();
        tokio::spawn(async move {
            let Ok(mut pubsub) = client.get_async_pubsub().await else {
                return;
            };
            if pubsub.subscribe(&channel).await.is_err() {
                return;
            }
            let mut stream = pubsub.into_on_message();
            while let Some(msg) = stream.next().await {
                if let Ok(payload) = msg.get_payload::<String>()
                    && let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&payload)
                {
                    // Err only means no current local subscribers.
                    let _ = sink.send(server_msg);
                }
            }
        });
        tx
    }
}

#[async_trait]
impl Fanout for RedisFanout {
    async fn publish(&self, session_id: &str, msg: ServerMessage) {
        let Ok(payload) = serde_json::to_string(&msg) else {
            return;
        };
        let mut conn = self.publisher.clone();
        let _: Result<i64, _> = conn.publish(channel_name(session_id), payload).await;
    }

    async fn subscribe(&self, session_id: &str) -> mpsc::UnboundedReceiver<ServerMessage> {
        // Ensure the session's Redis subscriber exists, then bridge the local
        // broadcast into an mpsc for the WS handler.
        let mut rx = self.local(session_id).subscribe();
        let (tx, out) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(m) => {
                        if tx.send(m).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        out
    }
}
