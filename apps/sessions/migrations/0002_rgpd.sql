-- Sessions RGPD surfaces (rgpd-kit first increment; design
-- docs/superpowers/specs/2026-07-23-rgpd-kit-first-increment-design.md §5).
-- Two per-context, tenant-scoped, append-only evidence tables — Sessions owns
-- its own erasure evidence; there is no cross-context table anywhere
-- (bounded-context hard rule, design §2).
--
-- session_deleted_subjects is the Art. 17 tombstone: inserting it inside the
-- accepted deletion transaction removes logical access for the subject while
-- the append-only event log awaits physical compaction through the retention
-- path (DATA-LIFECYCLE §Explicit deletion). session_subject_audit is the
-- append-only trail of data-subject requests. Both store ONLY opaque
-- sha-256 subject digests — never plaintext identifiers (design Appendix B) —
-- and `detail` carries refusal reason codes only, never free text or PII.
-- Grants exclude UPDATE and DELETE: evidence is immutable for the
-- application role, exactly like deletion_receipts (packages/data 0002).

CREATE TABLE session_deleted_subjects (
  tenant_id text NOT NULL
    CONSTRAINT session_deleted_subjects_tenant_format CHECK (tenant_id ~ '^ten_[a-z0-9]{16,64}$'),
  subject_digest text NOT NULL
    CONSTRAINT session_deleted_subjects_digest_format CHECK (subject_digest ~ '^[a-f0-9]{64}$'),
  receipt_id text NOT NULL,
  deleted_at timestamptz NOT NULL,
  -- One tombstone per subject per tenant: a second erasure of the same
  -- subject conflicts instead of forking the evidence.
  PRIMARY KEY (tenant_id, subject_digest)
);

ALTER TABLE session_deleted_subjects ENABLE ROW LEVEL SECURITY;
ALTER TABLE session_deleted_subjects FORCE ROW LEVEL SECURITY;

CREATE POLICY session_deleted_subjects_tenant_isolation ON session_deleted_subjects
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON session_deleted_subjects TO libre_ai_app;

CREATE TABLE session_subject_audit (
  tenant_id text NOT NULL
    CONSTRAINT session_subject_audit_tenant_format CHECK (tenant_id ~ '^ten_[a-z0-9]{16,64}$'),
  request_id text NOT NULL,
  subject_digest text NOT NULL
    CONSTRAINT session_subject_audit_digest_format CHECK (subject_digest ~ '^[a-f0-9]{64}$'),
  right_type text NOT NULL CONSTRAINT session_subject_audit_right_enum CHECK (right_type IN (
    'access', 'rectification', 'erasure', 'restriction', 'portability', 'object'
  )),
  status text NOT NULL CONSTRAINT session_subject_audit_status_enum CHECK (status IN (
    'received', 'acknowledged', 'in-progress', 'fulfilled', 'refused'
  )),
  -- Refusal reason codes only (e.g. 'sessions.rgpd.subject_unknown'):
  -- never free text, never PII.
  detail text,
  -- On the terminal row of a fulfilled erasure: the deletion-receipt id,
  -- joining the audit trail to the deletion evidence at the storage level
  -- (an auditor goes audit row -> receipt without the ephemeral HTTP body).
  receipt_id text,
  recorded_at timestamptz NOT NULL,
  -- One row per request state: a request is `received` once and reaches one
  -- terminal state once; replays conflict instead of forking the trail.
  PRIMARY KEY (tenant_id, request_id, status)
);

ALTER TABLE session_subject_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE session_subject_audit FORCE ROW LEVEL SECURITY;

CREATE POLICY session_subject_audit_tenant_isolation ON session_subject_audit
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON session_subject_audit TO libre_ai_app;
