-- Sessions v1 persistence (docs/apps/sessions.md §Data). PostgreSQL owns the
-- append-only session event log, tenant-scoped behind FORCE row-level security
-- keyed on the app.tenant_id GUC set by the withTenantDbTransaction barrier
-- (packages/data). The tenant-format CHECK and the least-privilege grant — which
-- excludes UPDATE and DELETE — are the structural floor: the causal log is
-- immutable and tenant-isolated even for a caller that bypasses the application
-- helpers. Depends on the libre_ai_app role (packages/data 0000_app_role.sql).

CREATE TABLE session_events (
  tenant_id text NOT NULL
    CONSTRAINT session_events_tenant_format CHECK (tenant_id ~ '^ten_[a-z0-9]{16,64}$'),
  session_id text NOT NULL,
  sequence integer NOT NULL
    CONSTRAINT session_events_sequence_positive CHECK (sequence >= 1),
  event_id text NOT NULL,
  revision integer NOT NULL
    CONSTRAINT session_events_revision_nonneg CHECK (revision >= 0),
  type text NOT NULL CONSTRAINT session_events_type_enum CHECK (type IN (
    'member-added', 'session-created', 'source-attached', 'participant-joined',
    'contribution-submitted', 'synthesis-drafted', 'outcome-approved',
    'outcome-rejected', 'session-closed', 'session-exported', 'session-deleted'
  )),
  actor_kind text NOT NULL CONSTRAINT session_events_actor_kind_enum
    CHECK (actor_kind IN ('human', 'provider', 'system')),
  actor_id text NOT NULL,
  occurred_at timestamptz NOT NULL,
  data jsonb NOT NULL,
  recorded_at timestamptz NOT NULL,
  -- The composite key is the causal ordering: one event per (tenant, session,
  -- sequence), so a duplicate sequence (a replay) conflicts rather than forks.
  PRIMARY KEY (tenant_id, session_id, sequence)
);

ALTER TABLE session_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE session_events FORCE ROW LEVEL SECURITY;

CREATE POLICY session_events_tenant_isolation ON session_events
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- Append-only: reported events are never rewritten (docs/apps/sessions.md — local
-- and public events are an authoritative append-only stream). The grant excludes
-- UPDATE and DELETE, so the log is immutable even to the application role.
GRANT SELECT, INSERT ON session_events TO libre_ai_app;
