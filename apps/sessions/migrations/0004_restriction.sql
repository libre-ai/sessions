-- Sessions Art. 18 restriction store (WP-G3-S01; restriction increment design
-- §3 (Option A)/§4). A third per-context, tenant-scoped, append-only evidence table, owned
-- entirely inside the Sessions bounded context — no cross-context table, same
-- hard rule as session_deleted_subjects / session_subject_audit (0002_rgpd.sql).
--
-- session_restricted_subjects holds the Art. 18 processing-restriction state as
-- an append-only history: the CURRENT state of a subject is the row with the
-- highest `entry_seq` for (tenant_id, subject_digest). A lift or a re-restriction
-- is a NEW row, never a rewrite — the grant excludes UPDATE and DELETE, exactly
-- like the two 0002 tables. `entry_seq` (an identity, not recorded_at) keys
-- recency so the current state is deterministic under a fixed injected clock:
-- same-timestamp writes still order by the monotonic identity, never ambiguous.
--
-- `ground` is one of the four Art. 18(1) grounds the SUBJECT supplies through
-- the request (never invented here); it is present on `restricted` rows and NULL
-- on the lift rows (`lift-pending`, `lifted`) — a lift's justification lives in
-- the audit trail, not in this state row. The CHECK enforces that invariant
-- structurally.
--
-- Audit-`detail` documentation, EXTENDING 0002 (which fixed "refusal reason
-- codes only"): the two-step Art. 18(3) lift writes to session_subject_audit
-- under a SYNTHETIC request_id `lift_<entry_seq>` (the entry_seq of the
-- `restricted` row being lifted), status `in-progress` then `fulfilled`, with
-- `detail` = 'sessions.rgpd.notice_required' then 'sessions.rgpd.notice_attested'
-- — still codes, never free text, never PII. Notice channel v1 is an owner
-- attestation recorded in that fulfilled audit row (owner decision 2026-07-24);
-- `confirmLift` is callable only once that notice obligation is discharged.
-- Both stores speak ONLY opaque sha-256 subject digests (design Appendix B).

CREATE TABLE session_restricted_subjects (
  tenant_id text NOT NULL
    CONSTRAINT session_restricted_subjects_tenant_format CHECK (tenant_id ~ '^ten_[a-z0-9]{16,64}$'),
  subject_digest text NOT NULL
    CONSTRAINT session_restricted_subjects_digest_format CHECK (subject_digest ~ '^[a-f0-9]{64}$'),
  -- Monotonic recency key: the current state is the max entry_seq per subject.
  entry_seq bigint GENERATED ALWAYS AS IDENTITY,
  state text NOT NULL
    CONSTRAINT session_restricted_subjects_state_enum CHECK (state IN ('restricted', 'lift-pending', 'lifted')),
  -- Art. 18(1) grounds; the subject supplies it, the implementation never does.
  ground text
    CONSTRAINT session_restricted_subjects_ground_enum CHECK (ground IN (
      'accuracy-contested', 'unlawful-opposed-erasure', 'needed-for-legal-claims', 'objection-pending'
    )),
  request_id text NOT NULL,
  recorded_at timestamptz NOT NULL,
  -- Append-only history keyed by the identity: one row per state transition,
  -- a replay of the same entry_seq conflicts instead of forking the history.
  PRIMARY KEY (tenant_id, subject_digest, entry_seq),
  -- The ground is present exactly on `restricted` rows and absent on lift rows.
  CONSTRAINT session_restricted_subjects_ground_iff_restricted CHECK (
    (state = 'restricted' AND ground IS NOT NULL)
    OR (state <> 'restricted' AND ground IS NULL)
  )
);

ALTER TABLE session_restricted_subjects ENABLE ROW LEVEL SECURITY;
ALTER TABLE session_restricted_subjects FORCE ROW LEVEL SECURITY;

CREATE POLICY session_restricted_subjects_tenant_isolation ON session_restricted_subjects
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- Append-only evidence: the grant excludes UPDATE and DELETE, so the
-- restriction history is immutable for the application role — a lift is a new
-- row, never a rewrite of the restricting row.
GRANT SELECT, INSERT ON session_restricted_subjects TO libre_ai_app;
