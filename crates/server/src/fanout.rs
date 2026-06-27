//! The fanout seam: deliver a session's server messages to every connection.
//!
//! Single-instance today ([`BroadcastFanout`], tokio broadcast); the Redis
//! pub/sub implementation ([`crate::redis_fanout`]) plugs in behind the same
//! trait to fan out across instances — this is the seam TB-2 exists to create.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{broadcast, mpsc};

use presto_core::protocol::ServerMessage;

/// Per-session broadcast buffer (2048, after the adversarial review observed
/// saturation at 1024 / 500 concurrent — see `docs/plans/`).
pub const BUFFER: usize = 2048;

/// Publish/subscribe of session messages, abstracted over transport.
#[async_trait]
pub trait Fanout: Send + Sync {
    /// Deliver `msg` to every current subscriber of `session_id`.
    async fn publish(&self, session_id: &str, msg: ServerMessage);
    /// Open an independent subscription to `session_id`'s messages.
    async fn subscribe(&self, session_id: &str) -> mpsc::UnboundedReceiver<ServerMessage>;
}

/// Single-instance fanout via per-session tokio broadcast channels.
#[derive(Default)]
pub struct BroadcastFanout {
    channels: Mutex<HashMap<String, broadcast::Sender<ServerMessage>>>,
}

impl BroadcastFanout {
    pub fn new() -> Self {
        Self::default()
    }

    fn sender(&self, session_id: &str) -> broadcast::Sender<ServerMessage> {
        self.channels
            .lock()
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(BUFFER).0)
            .clone()
    }
}

#[async_trait]
impl Fanout for BroadcastFanout {
    async fn publish(&self, session_id: &str, msg: ServerMessage) {
        // Err only means "no current subscribers", which is fine.
        let _ = self.sender(session_id).send(msg);
    }

    async fn subscribe(&self, session_id: &str) -> mpsc::UnboundedReceiver<ServerMessage> {
        let mut rx = self.sender(session_id).subscribe();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn published_message_reaches_a_subscriber() {
        let fanout = BroadcastFanout::new();
        let mut rx = fanout.subscribe("s1").await;
        fanout.publish("s1", ServerMessage::Pong).await;
        let got = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap();
        assert_eq!(got, Some(ServerMessage::Pong));
    }

    #[tokio::test]
    async fn subscribers_are_isolated_per_session() {
        let fanout = BroadcastFanout::new();
        let mut other = fanout.subscribe("s2").await;
        fanout.publish("s1", ServerMessage::Pong).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), other.recv())
                .await
                .is_err(),
            "a message for s1 must not reach an s2 subscriber"
        );
    }
}
