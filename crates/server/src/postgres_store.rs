//! Postgres-backed [`SessionStore`]: authoritative session state shared across
//! instances, so a `reveal` on instance A sees answers submitted on instance B.
//!
//! Runtime (non-macro) queries, so the crate compiles without a database. The
//! schema is created on connect.

use std::collections::BTreeMap;

use async_trait::async_trait;
use sqlx::Row;
use sqlx::postgres::PgPool;

use presto_core::protocol::{LeaderboardEntry, Question, QuestionPublic};

use crate::session::{ANSWER_GRACE_MS, RevealResult, SessionError, score};
use crate::store::{SessionStore, StoreError, StoreResult};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS presto_sessions (
    id               TEXT PRIMARY KEY,
    host_id          TEXT NOT NULL,
    phase            TEXT NOT NULL DEFAULT 'lobby',
    current_question TEXT,
    opened_at        BIGINT
);
CREATE TABLE IF NOT EXISTS presto_participants (
    session_id     TEXT   NOT NULL,
    participant_id TEXT   NOT NULL,
    name           TEXT   NOT NULL,
    score          BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, participant_id)
);
CREATE TABLE IF NOT EXISTS presto_answers (
    session_id     TEXT     NOT NULL,
    question_id    TEXT     NOT NULL,
    participant_id TEXT     NOT NULL,
    choice         SMALLINT NOT NULL,
    elapsed_ms     BIGINT   NOT NULL,
    PRIMARY KEY (session_id, question_id, participant_id)
);
-- Migration-safe: add opened_at to sessions tables created before this column existed.
ALTER TABLE presto_sessions ADD COLUMN IF NOT EXISTS opened_at BIGINT;
"#;

fn backend<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// Shared session state in Postgres.
pub struct PostgresSessionStore {
    pool: PgPool,
}

impl PostgresSessionStore {
    /// Connect and create the schema if needed.
    pub async fn connect(url: &str) -> StoreResult<Self> {
        let pool = PgPool::connect(url).await.map_err(backend)?;
        sqlx::raw_sql(SCHEMA)
            .execute(&pool)
            .await
            .map_err(backend)?;
        Ok(Self { pool })
    }

    async fn current_question(&self, session_id: &str) -> StoreResult<Option<Question>> {
        let row = sqlx::query("SELECT current_question FROM presto_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let raw: Option<String> = row.get("current_question");
        match raw {
            Some(json) => Ok(Some(serde_json::from_str(&json).map_err(backend)?)),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl SessionStore for PostgresSessionStore {
    async fn ensure(&self, session_id: &str, host_id: &str) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO presto_sessions (id, host_id, phase) VALUES ($1, $2, 'lobby') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(session_id)
        .bind(host_id)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn join(&self, session_id: &str, participant_id: &str, name: &str) -> StoreResult<u32> {
        sqlx::query(
            "INSERT INTO presto_participants (session_id, participant_id, name) \
             VALUES ($1, $2, $3) ON CONFLICT (session_id, participant_id) DO NOTHING",
        )
        .bind(session_id)
        .bind(participant_id)
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM presto_participants WHERE session_id = $1")
                .bind(session_id)
                .fetch_one(&self.pool)
                .await
                .map_err(backend)?;
        Ok(count as u32)
    }

    async fn push_question(
        &self,
        session_id: &str,
        question: &Question,
        opened_at_ms: u64,
    ) -> StoreResult<()> {
        let json = serde_json::to_string(question).map_err(backend)?;
        sqlx::query(
            "UPDATE presto_sessions SET current_question = $1, phase = 'asking', opened_at = $2 \
             WHERE id = $3",
        )
        .bind(json)
        // Epoch millis fit i64 for ~292M years; the cast cannot overflow.
        .bind(opened_at_ms as i64)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn submit_answer(
        &self,
        session_id: &str,
        participant_id: &str,
        choice: u8,
        now_ms: u64,
    ) -> StoreResult<()> {
        let row = sqlx::query("SELECT phase, opened_at FROM presto_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        let Some(row) = row else {
            return Err(StoreError::Session(SessionError::NotAsking));
        };
        let phase: String = row.get("phase");
        let opened_at: Option<i64> = row.get("opened_at");
        if phase != "asking" {
            return Err(StoreError::Session(SessionError::NotAsking));
        }
        let (Some(opened_at), Some(question)) =
            (opened_at, self.current_question(session_id).await?)
        else {
            return Err(StoreError::Session(SessionError::NotAsking));
        };

        // Server-side close deadline: timer plus a network-latency grace.
        let opened = opened_at as u64;
        let timer_ms = u64::from(question.timer_sec) * 1000;
        if now_ms > opened + timer_ms + ANSWER_GRACE_MS {
            return Err(StoreError::Session(SessionError::Closed));
        }

        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM presto_participants WHERE session_id = $1 AND participant_id = $2",
        )
        .bind(session_id)
        .bind(participant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(backend)?;
        if exists == 0 {
            return Err(StoreError::Session(SessionError::UnknownParticipant));
        }

        // Server times the answer; the client value (if any) is ignored.
        let elapsed_ms = u32::try_from(now_ms.saturating_sub(opened)).unwrap_or(u32::MAX);
        let inserted = sqlx::query(
            "INSERT INTO presto_answers (session_id, question_id, participant_id, choice, elapsed_ms) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (session_id, question_id, participant_id) DO NOTHING",
        )
        .bind(session_id)
        .bind(&question.id)
        .bind(participant_id)
        .bind(i16::from(choice))
        // BIGINT column: i64::from is lossless for the full u32 range (no wrap).
        .bind(i64::from(elapsed_ms))
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        if inserted.rows_affected() == 0 {
            return Err(StoreError::Session(SessionError::AlreadyAnswered));
        }
        Ok(())
    }

    async fn snapshot(&self, session_id: &str) -> StoreResult<Option<QuestionPublic>> {
        let phase: Option<String> =
            sqlx::query_scalar("SELECT phase FROM presto_sessions WHERE id = $1")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(backend)?;
        if phase.as_deref() != Some("asking") {
            return Ok(None);
        }
        Ok(self.current_question(session_id).await?.map(|q| q.public()))
    }

    async fn reveal(&self, session_id: &str) -> StoreResult<RevealResult> {
        let Some(question) = self.current_question(session_id).await? else {
            return Err(StoreError::Session(SessionError::NoQuestion));
        };
        let correct = question.correct_choice;

        let answers =
            sqlx::query("SELECT participant_id, choice, elapsed_ms FROM presto_answers WHERE session_id = $1 AND question_id = $2")
                .bind(session_id)
                .bind(&question.id)
                .fetch_all(&self.pool)
                .await
                .map_err(backend)?;

        let total = answers.len();
        let mut wrong = 0usize;
        for a in &answers {
            let choice: i16 = a.get("choice");
            let elapsed: i64 = a.get("elapsed_ms");
            if choice as u8 == correct {
                let pid: String = a.get("participant_id");
                let elapsed_ms = u32::try_from(elapsed).unwrap_or(u32::MAX);
                let points = i64::from(score(true, elapsed_ms));
                sqlx::query("UPDATE presto_participants SET score = score + $1 WHERE session_id = $2 AND participant_id = $3")
                    .bind(points)
                    .bind(session_id)
                    .bind(&pid)
                    .execute(&self.pool)
                    .await
                    .map_err(backend)?;
            } else {
                wrong += 1;
            }
        }

        let rows = sqlx::query(
            "SELECT participant_id, name, score FROM presto_participants WHERE session_id = $1 \
             ORDER BY score DESC, participant_id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        let leaderboard: Vec<LeaderboardEntry> = rows
            .iter()
            .map(|r| LeaderboardEntry {
                participant_id: r.get("participant_id"),
                name: r.get("name"),
                score: r.get::<i64, _>("score") as u32,
            })
            .collect();

        let confusion = if total > 0 {
            wrong as f32 / total as f32
        } else {
            0.0
        };
        let heatmap: BTreeMap<String, f32> = question
            .source_section_ids
            .iter()
            .map(|s| (s.clone(), confusion))
            .collect();

        sqlx::query("UPDATE presto_sessions SET phase = 'revealed' WHERE id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(backend)?;

        Ok(RevealResult {
            correct_choice: correct,
            leaderboard,
            heatmap,
        })
    }
}
