-- Product-local durable jobs and metadata-only outbox.
-- Apply with a migration role. Runtime roles need DML only and must not have
-- BYPASSRLS or superuser privileges.

CREATE TABLE IF NOT EXISTS presto_jobs (
    organization_id    TEXT NOT NULL,
    workspace_id       TEXT NOT NULL,
    job_id              TEXT NOT NULL,
    queue_seq           BIGSERIAL UNIQUE,
    kind                TEXT NOT NULL,
    idempotency_key     TEXT NOT NULL,
    state               TEXT NOT NULL,
    revision            BIGINT NOT NULL,
    attempts            INTEGER NOT NULL,
    max_attempts        INTEGER NOT NULL,
    lease_owner         TEXT,
    lease_expires_at_ms BIGINT,
    cancel_requested    BOOLEAN NOT NULL DEFAULT FALSE,
    result_ref          TEXT,
    failure_code        TEXT,
    PRIMARY KEY (organization_id, workspace_id, job_id),
    UNIQUE (organization_id, workspace_id, idempotency_key),
    CHECK (state IN ('queued', 'leased', 'succeeded', 'failed', 'cancelled')),
    CHECK (revision > 0),
    CHECK (attempts >= 0 AND attempts <= max_attempts),
    CHECK (max_attempts BETWEEN 1 AND 10),
    CHECK ((lease_owner IS NULL) = (lease_expires_at_ms IS NULL))
);

CREATE INDEX IF NOT EXISTS presto_jobs_lease_idx
    ON presto_jobs (organization_id, workspace_id, state, lease_expires_at_ms, queue_seq);

CREATE TABLE IF NOT EXISTS presto_job_events (
    event_seq           BIGSERIAL PRIMARY KEY,
    event_id            TEXT NOT NULL UNIQUE,
    organization_id     TEXT NOT NULL,
    workspace_id        TEXT NOT NULL,
    job_id               TEXT NOT NULL,
    revision             BIGINT NOT NULL,
    event_type           TEXT NOT NULL,
    claim_owner          TEXT,
    claim_id             TEXT,
    claim_expires_at_ms  BIGINT,
    delivery_attempts    INTEGER NOT NULL DEFAULT 0,
    published_at_ms      BIGINT,
    FOREIGN KEY (organization_id, workspace_id, job_id)
        REFERENCES presto_jobs (organization_id, workspace_id, job_id),
    CHECK (revision > 0),
    CHECK (delivery_attempts >= 0),
    CHECK (
        (claim_owner IS NULL AND claim_id IS NULL AND claim_expires_at_ms IS NULL)
        OR
        (claim_owner IS NOT NULL AND claim_id IS NOT NULL AND claim_expires_at_ms IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS presto_job_events_claim_idx
    ON presto_job_events (
        organization_id,
        workspace_id,
        published_at_ms,
        claim_expires_at_ms,
        event_seq
    );

ALTER TABLE presto_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE presto_jobs FORCE ROW LEVEL SECURITY;
ALTER TABLE presto_job_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE presto_job_events FORCE ROW LEVEL SECURITY;

DO $policy$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_policies
        WHERE schemaname = current_schema()
          AND tablename = 'presto_jobs'
          AND policyname = 'presto_jobs_tenant_scope'
    ) THEN
        CREATE POLICY presto_jobs_tenant_scope ON presto_jobs
            USING (
                organization_id = current_setting('presto.organization_id', true)
                AND workspace_id = current_setting('presto.workspace_id', true)
            )
            WITH CHECK (
                organization_id = current_setting('presto.organization_id', true)
                AND workspace_id = current_setting('presto.workspace_id', true)
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_policies
        WHERE schemaname = current_schema()
          AND tablename = 'presto_job_events'
          AND policyname = 'presto_job_events_tenant_scope'
    ) THEN
        CREATE POLICY presto_job_events_tenant_scope ON presto_job_events
            USING (
                organization_id = current_setting('presto.organization_id', true)
                AND workspace_id = current_setting('presto.workspace_id', true)
            )
            WITH CHECK (
                organization_id = current_setting('presto.organization_id', true)
                AND workspace_id = current_setting('presto.workspace_id', true)
            );
    END IF;
END
$policy$;
