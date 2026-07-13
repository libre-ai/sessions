//! Postgres-backed [`SessionStore`]: authoritative session state shared across
//! instances, so a `reveal` on instance A sees answers submitted on instance B.
//! Answer submissions lock the session row, validate the exact open question,
//! and reject malformed choices before any insert; opening a new question clears
//! the previous round's answers atomically; reveal is transactionally idempotent
//! and caches its exact result, so concurrent calls never double-score or drift.
//!
//! Runtime (non-macro) queries, so the crate compiles without a database. The
//! schema is created on connect.

use std::collections::BTreeMap;

use async_trait::async_trait;
use sqlx::Row;
use sqlx::postgres::PgPool;

use presto_core::protocol::{
    LeaderboardEntry, MAX_SESSION_SNAPSHOT_HEATMAP_ENTRIES, MAX_SESSION_SNAPSHOT_LEADERBOARD,
    MAX_SESSION_SNAPSHOT_PARTICIPANTS, ParticipantPublic, PublicReveal, Question, QuestionPublic,
    SessionPhasePublic, SessionSnapshot,
};

use crate::session::{
    ANSWER_GRACE_MS, RevealResult, SectionMastery, SessionError, is_correct, score,
};
use crate::store::{SessionStore, StoreError, StoreResult};

/// Encode/decode the selected choice indices as a comma-separated string.
fn encode_choices(choices: &[u8]) -> String {
    choices
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_choices(s: &str) -> Vec<u8> {
    s.split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect()
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS presto_sessions (
    id               TEXT PRIMARY KEY,
    host_id          TEXT NOT NULL,
    phase            TEXT NOT NULL DEFAULT 'lobby',
    current_question TEXT,
    opened_at        BIGINT,
    last_reveal      TEXT
);
CREATE TABLE IF NOT EXISTS presto_participants (
    session_id     TEXT   NOT NULL,
    participant_id TEXT   NOT NULL,
    name           TEXT   NOT NULL,
    score          BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, participant_id)
);
CREATE TABLE IF NOT EXISTS presto_answers (
    session_id     TEXT   NOT NULL,
    question_id    TEXT   NOT NULL,
    participant_id TEXT   NOT NULL,
    choices        TEXT   NOT NULL,
    elapsed_ms     BIGINT NOT NULL,
    PRIMARY KEY (session_id, question_id, participant_id)
);
CREATE TABLE IF NOT EXISTS presto_mastery (
    session_id     TEXT   NOT NULL,
    participant_id TEXT   NOT NULL,
    section_id     TEXT   NOT NULL,
    correct        BIGINT NOT NULL DEFAULT 0,
    total          BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, participant_id, section_id)
);
-- Migration-safe: add opened_at to sessions tables created before this column existed.
ALTER TABLE presto_sessions ADD COLUMN IF NOT EXISTS opened_at BIGINT;
ALTER TABLE presto_sessions ADD COLUMN IF NOT EXISTS last_reveal TEXT;
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
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let updated = sqlx::query(
            "UPDATE presto_sessions SET current_question = $1, phase = 'asking', opened_at = $2, last_reveal = NULL \
             WHERE id = $3 RETURNING id",
        )
        .bind(json)
        // Epoch millis fit i64 for ~292M years; the cast cannot overflow.
        .bind(opened_at_ms as i64)
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;
        if updated.is_some() {
            sqlx::query("DELETE FROM presto_answers WHERE session_id = $1")
                .bind(session_id)
                .execute(&mut *tx)
                .await
                .map_err(backend)?;
        }
        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    async fn submit_answer(
        &self,
        session_id: &str,
        participant_id: &str,
        question_id: &str,
        choices: Vec<u8>,
        now_ms: u64,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let row = sqlx::query(
            "SELECT phase, opened_at, current_question FROM presto_sessions WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;
        let Some(row) = row else {
            return Err(StoreError::Session(SessionError::AnswerClosed));
        };
        let phase: String = row.get("phase");
        if phase != "asking" {
            return Err(StoreError::Session(SessionError::AnswerClosed));
        }
        let opened_at: Option<i64> = row.get("opened_at");
        let raw_question: Option<String> = row.get("current_question");
        let (Some(opened_at), Some(raw_question)) = (opened_at, raw_question) else {
            return Err(StoreError::Session(SessionError::AnswerClosed));
        };
        let question: Question = serde_json::from_str(&raw_question).map_err(backend)?;

        // Server-side close deadline: timer plus a network-latency grace.
        let opened = opened_at as u64;
        let timer_ms = u64::from(question.timer_sec) * 1000;
        if now_ms > opened + timer_ms + ANSWER_GRACE_MS {
            return Err(StoreError::Session(SessionError::AnswerClosed));
        }
        if question.id != question_id {
            return Err(StoreError::Session(SessionError::WrongQuestion));
        }

        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM presto_participants WHERE session_id = $1 AND participant_id = $2",
        )
        .bind(session_id)
        .bind(participant_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(backend)?;
        if exists == 0 {
            return Err(StoreError::Session(SessionError::InvalidAnswer));
        }

        let already_answered: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM presto_answers WHERE session_id = $1 AND question_id = $2 AND participant_id = $3",
        )
        .bind(session_id)
        .bind(question_id)
        .bind(participant_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(backend)?;
        if already_answered > 0 {
            return Err(StoreError::Session(SessionError::AlreadyAnswered));
        }

        crate::session::Session::validate_answer_choices(&question, &choices)
            .map_err(StoreError::from)?;

        // Server times the answer; the client value (if any) is ignored.
        let elapsed_ms = u32::try_from(now_ms.saturating_sub(opened)).unwrap_or(u32::MAX);
        sqlx::query(
            "INSERT INTO presto_answers (session_id, question_id, participant_id, choices, elapsed_ms) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(session_id)
        .bind(question_id)
        .bind(participant_id)
        .bind(encode_choices(&choices))
        // BIGINT column: i64::from is lossless for the full u32 range (no wrap).
        .bind(i64::from(elapsed_ms))
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    async fn snapshot(&self, session_id: &str) -> StoreResult<Option<QuestionPublic>> {
        let row = sqlx::query("SELECT phase, current_question FROM presto_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let phase: String = row.get("phase");
        if phase != "asking" {
            return Ok(None);
        }
        let raw: Option<String> = row.get("current_question");
        Ok(match raw {
            Some(json) => {
                let question: Question = serde_json::from_str(&json).map_err(backend)?;
                Some(question.public())
            }
            None => None,
        })
    }

    async fn guest_snapshot(
        &self,
        session_id: &str,
        participant_id: &str,
    ) -> StoreResult<Option<SessionSnapshot>> {
        let row = sqlx::query(
            "SELECT phase, current_question, last_reveal, \
                COUNT(p.participant_id) AS participant_count, \
                COALESCE((SELECT json_agg(json_build_object('participant_id', participant_id, 'name', name) ORDER BY participant_id) FROM (SELECT participant_id, name FROM presto_participants WHERE session_id = $1 ORDER BY participant_id LIMIT 32) limited)::text, '[]') AS participants, \
                EXISTS(SELECT 1 FROM presto_answers a WHERE a.session_id = $1 AND a.participant_id = $2 AND a.question_id = (current_question::jsonb->>'id')) AS answered \
             FROM presto_sessions s \
             LEFT JOIN presto_participants p ON p.session_id = s.id \
             WHERE s.id = $1 \
             GROUP BY s.phase, s.current_question, s.last_reveal",
        )
        .bind(session_id)
        .bind(participant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;
        let Some(row) = row else {
            return Ok(None);
        };

        let phase: String = row.get("phase");
        let raw_question: Option<String> = row.get("current_question");
        let raw_last_reveal: Option<String> = row.get("last_reveal");
        let participants_count: i64 = row.get("participant_count");
        let participants_json: String = row.get("participants");
        let answered: bool = row.get("answered");
        let participants: Vec<ParticipantPublic> =
            serde_json::from_str(&participants_json).map_err(backend)?;
        let parsed_question: Option<Question> = match raw_question.as_ref() {
            Some(json) => Some(serde_json::from_str(json).map_err(backend)?),
            None => None,
        };
        let question = match phase.as_str() {
            "lobby" => None,
            "asking" | "revealed" => match parsed_question.as_ref() {
                Some(question) => Some(question.public()),
                None => {
                    return Err(StoreError::Backend(
                        "missing current question for active session".into(),
                    ));
                }
            },
            _ => return Err(StoreError::Backend(format!("unknown phase: {phase}"))),
        };
        let reveal = match phase.as_str() {
            "revealed" => {
                let raw_last_reveal = raw_last_reveal.ok_or_else(|| {
                    StoreError::Backend("missing cached reveal for revealed session".into())
                })?;
                let result: crate::session::RevealResult =
                    serde_json::from_str(&raw_last_reveal).map_err(backend)?;
                let question_id = parsed_question
                    .as_ref()
                    .map(|question| question.id.clone())
                    .ok_or_else(|| {
                        StoreError::Backend("missing question id for revealed session".into())
                    })?;
                Some(PublicReveal {
                    question_id,
                    correct_choices: result.correct_choices,
                    leaderboard: result
                        .leaderboard
                        .into_iter()
                        .take(MAX_SESSION_SNAPSHOT_LEADERBOARD)
                        .collect(),
                    heatmap: result
                        .heatmap
                        .into_iter()
                        .take(MAX_SESSION_SNAPSHOT_HEATMAP_ENTRIES)
                        .collect(),
                })
            }
            _ => None,
        };

        let participants_count = u32::try_from(participants_count).unwrap_or(u32::MAX);
        let participants = participants
            .into_iter()
            .take(MAX_SESSION_SNAPSHOT_PARTICIPANTS)
            .collect();
        SessionSnapshot::new(
            match phase.as_str() {
                "lobby" => SessionPhasePublic::Lobby,
                "asking" => SessionPhasePublic::Asking,
                "revealed" => SessionPhasePublic::Revealed,
                _ => unreachable!(),
            },
            participants,
            participants_count,
            question,
            answered,
            reveal,
        )
        .map(Some)
        .map_err(StoreError::Backend)
    }

    async fn exists(&self, session_id: &str) -> StoreResult<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM presto_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&self.pool)
            .await
            .map_err(backend)?;
        Ok(count > 0)
    }

    async fn mastery(
        &self,
        session_id: &str,
        participant_id: &str,
    ) -> StoreResult<Vec<SectionMastery>> {
        let rows = sqlx::query(
            "SELECT section_id, correct, total FROM presto_mastery \
             WHERE session_id = $1 AND participant_id = $2 ORDER BY section_id",
        )
        .bind(session_id)
        .bind(participant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        Ok(rows
            .iter()
            .map(|r| SectionMastery {
                section_id: r.get("section_id"),
                correct: u32::try_from(r.get::<i64, _>("correct")).unwrap_or(u32::MAX),
                total: u32::try_from(r.get::<i64, _>("total")).unwrap_or(u32::MAX),
            })
            .collect())
    }

    async fn reveal(&self, session_id: &str) -> StoreResult<RevealResult> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let row = sqlx::query(
            "SELECT phase, current_question, last_reveal FROM presto_sessions WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;
        let Some(row) = row else {
            return Err(StoreError::Session(SessionError::NoQuestion));
        };
        let phase: String = row.get("phase");
        let raw_question: Option<String> = row.get("current_question");
        let raw_last_reveal: Option<String> = row.get("last_reveal");

        if phase == "revealed" {
            let raw_last_reveal = raw_last_reveal.ok_or_else(|| {
                StoreError::Backend("missing cached reveal for revealed session".into())
            })?;
            return serde_json::from_str(&raw_last_reveal).map_err(backend);
        }

        let Some(raw_question) = raw_question else {
            return Err(StoreError::Session(SessionError::NoQuestion));
        };
        let question: Question = serde_json::from_str(&raw_question).map_err(backend)?;
        let correct = question.correct_choices.clone();

        let answers = sqlx::query(
            "SELECT participant_id, choices, elapsed_ms FROM presto_answers WHERE session_id = $1 AND question_id = $2",
        )
        .bind(session_id)
        .bind(&question.id)
        .fetch_all(&mut *tx)
        .await
        .map_err(backend)?;

        for a in &answers {
            let pid: String = a.get("participant_id");
            let submitted = decode_choices(&a.get::<String, _>("choices"));
            let elapsed: i64 = a.get("elapsed_ms");
            if is_correct(&submitted, &correct) {
                let elapsed_ms = u32::try_from(elapsed).unwrap_or(u32::MAX);
                let points = i64::from(score(true, elapsed_ms));
                sqlx::query(
                    "UPDATE presto_participants SET score = score + $1 WHERE session_id = $2 AND participant_id = $3",
                )
                .bind(points)
                .bind(session_id)
                .bind(&pid)
                .execute(&mut *tx)
                .await
                .map_err(backend)?;
            }
            let inc = if is_correct(&submitted, &correct) {
                1_i64
            } else {
                0_i64
            };
            for section in &question.source_section_ids {
                sqlx::query(
                    "INSERT INTO presto_mastery (session_id, participant_id, section_id, correct, total) \
                     VALUES ($1, $2, $3, $4, 1) \
                     ON CONFLICT (session_id, participant_id, section_id) \
                     DO UPDATE SET correct = presto_mastery.correct + $4, total = presto_mastery.total + 1",
                )
                .bind(session_id)
                .bind(&pid)
                .bind(section)
                .bind(inc)
                .execute(&mut *tx)
                .await
                .map_err(backend)?;
            }
        }

        let rows = sqlx::query(
            "SELECT participant_id, name, score FROM presto_participants WHERE session_id = $1 \
             ORDER BY score DESC, participant_id ASC",
        )
        .bind(session_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(backend)?;
        let leaderboard: Vec<LeaderboardEntry> = rows
            .iter()
            .map(|r| LeaderboardEntry {
                participant_id: r.get("participant_id"),
                name: r.get("name"),
                score: u32::try_from(r.get::<i64, _>("score")).unwrap_or(u32::MAX),
            })
            .collect();

        let total = answers.len();
        let wrong = answers
            .iter()
            .filter(|a| {
                let submitted = decode_choices(&a.get::<String, _>("choices"));
                !is_correct(&submitted, &correct)
            })
            .count();
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
        let result = RevealResult {
            correct_choices: correct,
            leaderboard,
            heatmap,
        };
        let serialized = serde_json::to_string(&result).map_err(backend)?;

        sqlx::query(
            "UPDATE presto_sessions SET phase = 'revealed', last_reveal = $2 WHERE id = $1",
        )
        .bind(session_id)
        .bind(&serialized)
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
        tx.commit().await.map_err(backend)?;

        Ok(result)
    }
}
