//! Persistence for spaced-repetition flashcard decks (Prod2).
//!
//! A live session generates a deck from the sections a participant struggled
//! with; the cross-day SRS review loop lives outside the session, so the deck
//! must survive it. Decks are keyed by `owner` (the participant's stable subject)
//! and persisted with their SM-2 state for an external scheduler.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use sqlx::Row;
use sqlx::postgres::PgPool;

use presto_core::protocol::Flashcard;

use crate::store::{StoreError, StoreResult};

fn backend<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// Persist and retrieve a participant's flashcard deck across sessions.
#[async_trait]
pub trait FlashcardStore: Send + Sync {
    /// Persist `owner`'s deck, replacing any prior deck for them.
    async fn save_deck(&self, owner: &str, deck: &[Flashcard]) -> StoreResult<()>;
    /// Load `owner`'s deck (empty if none persisted), preserving order.
    async fn load_deck(&self, owner: &str) -> StoreResult<Vec<Flashcard>>;
}

/// Single-instance, in-memory deck store (tests / local).
#[derive(Default)]
pub struct InMemoryFlashcardStore {
    decks: Mutex<HashMap<String, Vec<Flashcard>>>,
}

impl InMemoryFlashcardStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl FlashcardStore for InMemoryFlashcardStore {
    async fn save_deck(&self, owner: &str, deck: &[Flashcard]) -> StoreResult<()> {
        self.decks.lock().insert(owner.to_string(), deck.to_vec());
        Ok(())
    }

    async fn load_deck(&self, owner: &str) -> StoreResult<Vec<Flashcard>> {
        Ok(self.decks.lock().get(owner).cloned().unwrap_or_default())
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS presto_flashcards (
    owner         TEXT NOT NULL,
    ordinal       INT  NOT NULL,
    section_id    TEXT NOT NULL,
    front         TEXT NOT NULL,
    back          TEXT NOT NULL,
    ease_factor   REAL NOT NULL,
    interval_days INT  NOT NULL,
    PRIMARY KEY (owner, ordinal)
);
"#;

/// Multi-instance deck store in Postgres.
pub struct PostgresFlashcardStore {
    pool: PgPool,
}

impl PostgresFlashcardStore {
    pub async fn connect(url: &str) -> StoreResult<Self> {
        let pool = PgPool::connect(url).await.map_err(backend)?;
        sqlx::raw_sql(SCHEMA)
            .execute(&pool)
            .await
            .map_err(backend)?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl FlashcardStore for PostgresFlashcardStore {
    async fn save_deck(&self, owner: &str, deck: &[Flashcard]) -> StoreResult<()> {
        sqlx::query("DELETE FROM presto_flashcards WHERE owner = $1")
            .bind(owner)
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        for (i, c) in deck.iter().enumerate() {
            sqlx::query(
                "INSERT INTO presto_flashcards \
                   (owner, ordinal, section_id, front, back, ease_factor, interval_days) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(owner)
            .bind(i as i32)
            .bind(&c.section_id)
            .bind(&c.front)
            .bind(&c.back)
            .bind(c.ease_factor)
            .bind(c.interval_days as i32)
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        }
        Ok(())
    }

    async fn load_deck(&self, owner: &str) -> StoreResult<Vec<Flashcard>> {
        let rows = sqlx::query(
            "SELECT section_id, front, back, ease_factor, interval_days \
             FROM presto_flashcards WHERE owner = $1 ORDER BY ordinal",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        Ok(rows
            .iter()
            .map(|r| Flashcard {
                section_id: r.get("section_id"),
                front: r.get("front"),
                back: r.get("back"),
                ease_factor: r.get::<f32, _>("ease_factor"),
                interval_days: r.get::<i32, _>("interval_days") as u32,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck() -> Vec<Flashcard> {
        vec![
            Flashcard {
                section_id: "doc#p0".into(),
                front: "What is X?".into(),
                back: "X is Y.".into(),
                ease_factor: 2.5,
                interval_days: 0,
            },
            Flashcard {
                section_id: "doc#p1".into(),
                front: "Define Z".into(),
                back: "Z is W.".into(),
                ease_factor: 2.6,
                interval_days: 3,
            },
        ]
    }

    #[tokio::test]
    async fn in_memory_deck_persists_and_retrieves() {
        let store = InMemoryFlashcardStore::new();
        assert!(store.load_deck("u1").await.unwrap().is_empty());
        store.save_deck("u1", &deck()).await.unwrap();
        let loaded = store.load_deck("u1").await.unwrap();
        assert_eq!(loaded, deck());
        // Another owner's deck is independent.
        assert!(store.load_deck("u2").await.unwrap().is_empty());
    }
}
