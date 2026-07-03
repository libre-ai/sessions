-- I2: Initialize session and session_answers tables.
-- Postgres backend for SessionStore trait.

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    host_actor_id TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at TIMESTAMPTZ,
    state TEXT NOT NULL DEFAULT 'active',  -- 'active', 'ended', 'error'
    metadata JSONB DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS session_answers (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    participant_actor_id TEXT NOT NULL,
    question_id TEXT NOT NULL,
    choice TEXT NOT NULL,
    elapsed_ms INTEGER NOT NULL,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    score INTEGER,
    UNIQUE(session_id, participant_actor_id, question_id)
);

CREATE INDEX idx_session_answers_session ON session_answers(session_id);
CREATE INDEX idx_session_answers_participant ON session_answers(participant_actor_id);
