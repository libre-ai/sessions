//! PostgreSQL adapter for product-local jobs and the metadata-only outbox.
//!
//! Every operation opens a transaction and installs organization/workspace
//! settings consumed by forced RLS policies. The adapter never accepts an
//! unscoped query and never persists job payloads, prompts, or document text.

use async_trait::async_trait;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::jobs::{
    CompleteJob, Completion, EnqueueJob, Heartbeat, JobError, JobEvent, JobRecord, JobState,
    JobStore, MAX_JOB_LEASE_MS, MAX_OUTBOX_LEASE_MS, OutboxClaim, safe_id, safe_timestamp,
    validate_completion, validate_enqueue, validate_lease_guard, verify_lease,
};

const SCHEMA: &str = include_str!("../migrations/0001_jobs_and_outbox.sql");

/// Durable multi-instance job store. Schema application remains an explicit
/// operator action; [`connect`](Self::connect) does not mutate the database.
pub struct PostgresJobStore {
    pool: PgPool,
}

impl PostgresJobStore {
    pub async fn connect(url: &str) -> Result<Self, JobError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .map_err(internal)?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Apply the idempotent schema with a migration role. Production runtime
    /// roles must not be superusers and must not hold `BYPASSRLS`.
    pub async fn apply_schema(pool: &PgPool) -> Result<(), JobError> {
        sqlx::raw_sql(SCHEMA)
            .execute(pool)
            .await
            .map_err(internal)?;
        Ok(())
    }

    async fn scoped_transaction(
        &self,
        organization_id: &str,
        workspace_id: &str,
    ) -> Result<Transaction<'_, Postgres>, JobError> {
        validate_scope(organization_id, workspace_id)?;
        let mut transaction = self.pool.begin().await.map_err(internal)?;
        sqlx::query(
            "SELECT set_config('presto.organization_id', $1, true), \
                    set_config('presto.workspace_id', $2, true)",
        )
        .bind(organization_id)
        .bind(workspace_id)
        .execute(&mut *transaction)
        .await
        .map_err(internal)?;
        Ok(transaction)
    }
}

#[async_trait]
impl JobStore for PostgresJobStore {
    async fn enqueue(&self, request: EnqueueJob) -> Result<JobRecord, JobError> {
        validate_enqueue(&request)?;
        let mut transaction = self
            .scoped_transaction(&request.organization_id, &request.workspace_id)
            .await?;
        let job_id = format!("job_{}", Uuid::new_v4().simple());
        let inserted = sqlx::query(
            "INSERT INTO presto_jobs \
             (organization_id, workspace_id, job_id, kind, idempotency_key, state, revision, attempts, max_attempts) \
             VALUES ($1, $2, $3, $4, $5, 'queued', 1, 0, $6) \
             ON CONFLICT (organization_id, workspace_id, idempotency_key) DO NOTHING \
             RETURNING organization_id, workspace_id, job_id, kind, idempotency_key, state, \
             revision, attempts, max_attempts, lease_owner, lease_expires_at_ms, \
             cancel_requested, result_ref, failure_code",
        )
            .bind(&request.organization_id)
            .bind(&request.workspace_id)
            .bind(job_id)
            .bind(&request.kind)
            .bind(&request.idempotency_key)
            .bind(i32::try_from(request.max_attempts).map_err(|_| JobError::InvalidInput)?)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(internal)?;

        let record = if let Some(row) = inserted {
            let record = job_from_row(&row)?;
            append_event(
                &mut transaction,
                &record,
                "job_queued",
                &request.actor_ref,
                request.now_ms,
            )
            .await?;
            record
        } else {
            let row = sqlx::query(
                "SELECT organization_id, workspace_id, job_id, kind, idempotency_key, state, \
                 revision, attempts, max_attempts, lease_owner, lease_expires_at_ms, \
                 cancel_requested, result_ref, failure_code FROM presto_jobs \
                 WHERE organization_id = $1 AND workspace_id = $2 AND idempotency_key = $3",
            )
            .bind(&request.organization_id)
            .bind(&request.workspace_id)
            .bind(&request.idempotency_key)
            .fetch_one(&mut *transaction)
            .await
            .map_err(internal)?;
            job_from_row(&row)?
        };
        transaction.commit().await.map_err(internal)?;
        Ok(record)
    }

    async fn lease_next(
        &self,
        organization_id: &str,
        workspace_id: &str,
        worker_id: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<Option<JobRecord>, JobError> {
        if !safe_id(worker_id)
            || !safe_timestamp(now_ms)
            || !(1..=MAX_JOB_LEASE_MS).contains(&lease_ms)
        {
            return Err(JobError::InvalidInput);
        }
        let expires = now_ms.checked_add(lease_ms).ok_or(JobError::InvalidInput)?;
        let now = as_i64(now_ms)?;
        let expires = as_i64(expires)?;
        let mut transaction = self
            .scoped_transaction(organization_id, workspace_id)
            .await?;

        let exhausted = sqlx::query(
            "UPDATE presto_jobs SET state = 'failed', revision = revision + 1, \
                 lease_owner = NULL, lease_expires_at_ms = NULL, failure_code = 'lease_attempts_exhausted' \
             WHERE organization_id = $1 AND workspace_id = $2 AND state = 'leased' \
               AND lease_expires_at_ms <= $3 AND attempts >= max_attempts \
             RETURNING organization_id, workspace_id, job_id, kind, idempotency_key, state, \
             revision, attempts, max_attempts, lease_owner, lease_expires_at_ms, \
             cancel_requested, result_ref, failure_code",
        )
            .bind(organization_id)
            .bind(workspace_id)
            .bind(now)
            .fetch_all(&mut *transaction)
            .await
            .map_err(internal)?;
        for row in exhausted {
            let record = job_from_row(&row)?;
            append_event(
                &mut transaction,
                &record,
                "job_failed",
                "jobs_runtime",
                now_ms,
            )
            .await?;
        }

        let candidate = sqlx::query(
            "SELECT organization_id, workspace_id, job_id, kind, idempotency_key, state, \
             revision, attempts, max_attempts, lease_owner, lease_expires_at_ms, \
             cancel_requested, result_ref, failure_code FROM presto_jobs \
             WHERE organization_id = $1 AND workspace_id = $2 AND attempts < max_attempts \
               AND (state = 'queued' OR (state = 'leased' AND lease_expires_at_ms <= $3)) \
             ORDER BY queue_seq FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .bind(organization_id)
        .bind(workspace_id)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(internal)?;
        let Some(row) = candidate else {
            transaction.commit().await.map_err(internal)?;
            return Ok(None);
        };
        let mut record = job_from_row(&row)?;
        record.state = JobState::Leased;
        record.revision = record.revision.checked_add(1).ok_or(JobError::Internal)?;
        record.attempts = record.attempts.checked_add(1).ok_or(JobError::Internal)?;
        record.lease_owner = Some(worker_id.to_string());
        record.lease_expires_at_ms = Some(u64::try_from(expires).map_err(|_| JobError::Internal)?);
        record.failure_code = None;
        save_job(&mut transaction, &record).await?;
        append_event(&mut transaction, &record, "job_leased", worker_id, now_ms).await?;
        transaction.commit().await.map_err(internal)?;
        Ok(Some(record))
    }

    async fn heartbeat(&self, heartbeat: Heartbeat) -> Result<JobRecord, JobError> {
        validate_lease_guard(&heartbeat.lease)?;
        if !(1..=MAX_JOB_LEASE_MS).contains(&heartbeat.extend_by_ms) {
            return Err(JobError::InvalidInput);
        }
        let expires = heartbeat
            .lease
            .now_ms
            .checked_add(heartbeat.extend_by_ms)
            .ok_or(JobError::InvalidInput)?;
        as_i64(expires)?;
        let mut transaction = self
            .scoped_transaction(
                &heartbeat.lease.organization_id,
                &heartbeat.lease.workspace_id,
            )
            .await?;
        let mut record = lock_job(
            &mut transaction,
            &heartbeat.lease.organization_id,
            &heartbeat.lease.workspace_id,
            &heartbeat.lease.job_id,
        )
        .await?;
        verify_lease(
            &record,
            &heartbeat.lease.worker_id,
            heartbeat.lease.expected_revision,
            heartbeat.lease.now_ms,
        )?;
        record.revision = record.revision.checked_add(1).ok_or(JobError::Internal)?;
        record.lease_expires_at_ms = Some(expires);
        save_job(&mut transaction, &record).await?;
        append_event(
            &mut transaction,
            &record,
            "job_heartbeat",
            &heartbeat.lease.worker_id,
            heartbeat.lease.now_ms,
        )
        .await?;
        transaction.commit().await.map_err(internal)?;
        Ok(record)
    }

    async fn request_cancel(
        &self,
        organization_id: &str,
        workspace_id: &str,
        job_id: &str,
        actor_ref: &str,
        now_ms: u64,
    ) -> Result<JobRecord, JobError> {
        if !safe_id(job_id) || !safe_id(actor_ref) || !safe_timestamp(now_ms) {
            return Err(JobError::InvalidInput);
        }
        let mut transaction = self
            .scoped_transaction(organization_id, workspace_id)
            .await?;
        let mut record = lock_job(&mut transaction, organization_id, workspace_id, job_id).await?;
        match record.state {
            JobState::Queued => record.state = JobState::Cancelled,
            JobState::Leased => record.cancel_requested = true,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled => {
                return Err(JobError::WrongState);
            }
        }
        record.revision = record.revision.checked_add(1).ok_or(JobError::Internal)?;
        save_job(&mut transaction, &record).await?;
        append_event(
            &mut transaction,
            &record,
            "job_cancel_requested",
            actor_ref,
            now_ms,
        )
        .await?;
        transaction.commit().await.map_err(internal)?;
        Ok(record)
    }

    async fn complete(&self, request: CompleteJob) -> Result<JobRecord, JobError> {
        validate_lease_guard(&request.lease)?;
        validate_completion(&request.completion)?;
        let mut transaction = self
            .scoped_transaction(&request.lease.organization_id, &request.lease.workspace_id)
            .await?;
        let mut record = lock_job(
            &mut transaction,
            &request.lease.organization_id,
            &request.lease.workspace_id,
            &request.lease.job_id,
        )
        .await?;
        verify_lease(
            &record,
            &request.lease.worker_id,
            request.lease.expected_revision,
            request.lease.now_ms,
        )?;
        record.revision = record.revision.checked_add(1).ok_or(JobError::Internal)?;
        record.lease_owner = None;
        record.lease_expires_at_ms = None;
        let event_type = if record.cancel_requested {
            record.state = JobState::Cancelled;
            record.result_ref = None;
            record.failure_code = None;
            "job_cancelled"
        } else {
            match request.completion {
                Completion::Succeeded { result_ref } => {
                    record.state = JobState::Succeeded;
                    record.result_ref = Some(result_ref);
                    record.failure_code = None;
                    "job_succeeded"
                }
                Completion::Failed {
                    failure_code,
                    retryable,
                } => {
                    record.failure_code = Some(failure_code);
                    record.state = if retryable && record.attempts < record.max_attempts {
                        JobState::Queued
                    } else {
                        JobState::Failed
                    };
                    if record.state == JobState::Queued {
                        "job_retry_queued"
                    } else {
                        "job_failed"
                    }
                }
            }
        };
        save_job(&mut transaction, &record).await?;
        append_event(
            &mut transaction,
            &record,
            event_type,
            &request.lease.worker_id,
            request.lease.now_ms,
        )
        .await?;
        transaction.commit().await.map_err(internal)?;
        Ok(record)
    }

    async fn events(
        &self,
        organization_id: &str,
        workspace_id: &str,
        limit: u32,
    ) -> Result<Vec<JobEvent>, JobError> {
        if !(1..=1_000).contains(&limit) {
            return Err(JobError::InvalidInput);
        }
        let mut transaction = self
            .scoped_transaction(organization_id, workspace_id)
            .await?;
        let rows = sqlx::query(
            "SELECT event_id, organization_id, workspace_id, job_id, revision, event_type, \
             actor_ref, occurred_at_ms FROM presto_job_events WHERE organization_id = $1 AND workspace_id = $2 \
             ORDER BY event_seq DESC LIMIT $3",
        )
        .bind(organization_id)
        .bind(workspace_id)
        .bind(i64::from(limit))
        .fetch_all(&mut *transaction)
        .await
        .map_err(internal)?;
        let mut events = rows
            .iter()
            .map(event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        events.reverse();
        transaction.commit().await.map_err(internal)?;
        Ok(events)
    }

    async fn claim_events(
        &self,
        organization_id: &str,
        workspace_id: &str,
        publisher_id: &str,
        now_ms: u64,
        lease_ms: u64,
        limit: u32,
    ) -> Result<Vec<OutboxClaim>, JobError> {
        if !safe_id(publisher_id)
            || !safe_timestamp(now_ms)
            || !(1..=MAX_OUTBOX_LEASE_MS).contains(&lease_ms)
            || !(1..=100).contains(&limit)
        {
            return Err(JobError::InvalidInput);
        }
        let expires = now_ms.checked_add(lease_ms).ok_or(JobError::InvalidInput)?;
        let now = as_i64(now_ms)?;
        let expires_i64 = as_i64(expires)?;
        let mut transaction = self
            .scoped_transaction(organization_id, workspace_id)
            .await?;
        let rows = sqlx::query(
            "SELECT event_id, organization_id, workspace_id, job_id, revision, event_type, \
             actor_ref, occurred_at_ms FROM presto_job_events \
             WHERE organization_id = $1 AND workspace_id = $2 AND published_at_ms IS NULL \
               AND (claim_expires_at_ms IS NULL OR claim_expires_at_ms <= $3) \
             ORDER BY event_seq FOR UPDATE SKIP LOCKED LIMIT $4",
        )
        .bind(organization_id)
        .bind(workspace_id)
        .bind(now)
        .bind(i64::from(limit))
        .fetch_all(&mut *transaction)
        .await
        .map_err(internal)?;
        let mut claims = Vec::with_capacity(rows.len());
        for row in rows {
            let event = event_from_row(&row)?;
            let claim_id = format!("claim_{}", Uuid::new_v4().simple());
            let delivery_attempt: i32 = sqlx::query_scalar(
                "UPDATE presto_job_events SET claim_owner = $1, claim_id = $2, \
                     claim_expires_at_ms = $3, delivery_attempts = delivery_attempts + 1 \
                 WHERE organization_id = $4 AND workspace_id = $5 AND event_id = $6 \
                 RETURNING delivery_attempts",
            )
            .bind(publisher_id)
            .bind(&claim_id)
            .bind(expires_i64)
            .bind(organization_id)
            .bind(workspace_id)
            .bind(&event.event_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(internal)?;
            claims.push(OutboxClaim {
                event,
                publisher_id: publisher_id.to_string(),
                claim_id,
                claim_expires_at_ms: expires,
                delivery_attempt: u32::try_from(delivery_attempt)
                    .map_err(|_| JobError::Internal)?,
            });
        }
        transaction.commit().await.map_err(internal)?;
        Ok(claims)
    }

    async fn acknowledge_event(
        &self,
        organization_id: &str,
        workspace_id: &str,
        event_id: &str,
        publisher_id: &str,
        claim_id: &str,
        now_ms: u64,
    ) -> Result<(), JobError> {
        if !safe_id(event_id)
            || !safe_id(publisher_id)
            || !safe_id(claim_id)
            || !safe_timestamp(now_ms)
        {
            return Err(JobError::InvalidInput);
        }
        let now = as_i64(now_ms)?;
        let mut transaction = self
            .scoped_transaction(organization_id, workspace_id)
            .await?;
        let updated = sqlx::query(
            "UPDATE presto_job_events SET published_at_ms = $1, published_by = $5, claim_owner = NULL, \
                 claim_id = NULL, claim_expires_at_ms = NULL \
             WHERE organization_id = $2 AND workspace_id = $3 AND event_id = $4 \
               AND published_at_ms IS NULL AND claim_owner = $5 AND claim_id = $6 \
               AND claim_expires_at_ms > $1",
        )
        .bind(now)
        .bind(organization_id)
        .bind(workspace_id)
        .bind(event_id)
        .bind(publisher_id)
        .bind(claim_id)
        .execute(&mut *transaction)
        .await
        .map_err(internal)?;
        if updated.rows_affected() == 0 {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM presto_job_events \
                 WHERE organization_id = $1 AND workspace_id = $2 AND event_id = $3)",
            )
            .bind(organization_id)
            .bind(workspace_id)
            .bind(event_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(internal)?;
            return Err(if exists {
                JobError::OutboxClaimMismatch
            } else {
                JobError::NotFound
            });
        }
        transaction.commit().await.map_err(internal)?;
        Ok(())
    }
}

async fn lock_job(
    transaction: &mut Transaction<'_, Postgres>,
    organization_id: &str,
    workspace_id: &str,
    job_id: &str,
) -> Result<JobRecord, JobError> {
    let row = sqlx::query(
        "SELECT organization_id, workspace_id, job_id, kind, idempotency_key, state, \
         revision, attempts, max_attempts, lease_owner, lease_expires_at_ms, \
         cancel_requested, result_ref, failure_code FROM presto_jobs \
         WHERE organization_id = $1 AND workspace_id = $2 AND job_id = $3 FOR UPDATE",
    )
    .bind(organization_id)
    .bind(workspace_id)
    .bind(job_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(internal)?
    .ok_or(JobError::NotFound)?;
    job_from_row(&row)
}

async fn save_job(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
) -> Result<(), JobError> {
    let updated = sqlx::query(
        "UPDATE presto_jobs SET state = $1, revision = $2, attempts = $3, max_attempts = $4, \
             lease_owner = $5, lease_expires_at_ms = $6, cancel_requested = $7, \
             result_ref = $8, failure_code = $9 \
         WHERE organization_id = $10 AND workspace_id = $11 AND job_id = $12",
    )
    .bind(state_name(record.state))
    .bind(as_i64(record.revision)?)
    .bind(i32::try_from(record.attempts).map_err(|_| JobError::Internal)?)
    .bind(i32::try_from(record.max_attempts).map_err(|_| JobError::Internal)?)
    .bind(&record.lease_owner)
    .bind(record.lease_expires_at_ms.map(as_i64).transpose()?)
    .bind(record.cancel_requested)
    .bind(&record.result_ref)
    .bind(&record.failure_code)
    .bind(&record.organization_id)
    .bind(&record.workspace_id)
    .bind(&record.job_id)
    .execute(&mut **transaction)
    .await
    .map_err(internal)?;
    if updated.rows_affected() == 1 {
        Ok(())
    } else {
        Err(JobError::StaleRevision)
    }
}

async fn append_event(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
    event_type: &str,
    actor_ref: &str,
    occurred_at_ms: u64,
) -> Result<(), JobError> {
    sqlx::query(
        "INSERT INTO presto_job_events \
         (event_id, organization_id, workspace_id, job_id, revision, event_type, actor_ref, occurred_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(format!("evt_{}", Uuid::new_v4().simple()))
    .bind(&record.organization_id)
    .bind(&record.workspace_id)
    .bind(&record.job_id)
    .bind(as_i64(record.revision)?)
    .bind(event_type)
    .bind(actor_ref)
    .bind(as_i64(occurred_at_ms)?)
    .execute(&mut **transaction)
    .await
    .map_err(internal)?;
    Ok(())
}

fn job_from_row(row: &PgRow) -> Result<JobRecord, JobError> {
    let state: String = row.try_get("state").map_err(internal)?;
    Ok(JobRecord {
        organization_id: row.try_get("organization_id").map_err(internal)?,
        workspace_id: row.try_get("workspace_id").map_err(internal)?,
        job_id: row.try_get("job_id").map_err(internal)?,
        kind: row.try_get("kind").map_err(internal)?,
        idempotency_key: row.try_get("idempotency_key").map_err(internal)?,
        state: parse_state(&state)?,
        revision: from_i64(row.try_get("revision").map_err(internal)?)?,
        attempts: from_i32(row.try_get("attempts").map_err(internal)?)?,
        max_attempts: from_i32(row.try_get("max_attempts").map_err(internal)?)?,
        lease_owner: row.try_get("lease_owner").map_err(internal)?,
        lease_expires_at_ms: row
            .try_get::<Option<i64>, _>("lease_expires_at_ms")
            .map_err(internal)?
            .map(from_i64)
            .transpose()?,
        cancel_requested: row.try_get("cancel_requested").map_err(internal)?,
        result_ref: row.try_get("result_ref").map_err(internal)?,
        failure_code: row.try_get("failure_code").map_err(internal)?,
    })
}

fn event_from_row(row: &PgRow) -> Result<JobEvent, JobError> {
    Ok(JobEvent {
        event_id: row.try_get("event_id").map_err(internal)?,
        organization_id: row.try_get("organization_id").map_err(internal)?,
        workspace_id: row.try_get("workspace_id").map_err(internal)?,
        job_id: row.try_get("job_id").map_err(internal)?,
        revision: from_i64(row.try_get("revision").map_err(internal)?)?,
        event_type: row.try_get("event_type").map_err(internal)?,
        actor_ref: row.try_get("actor_ref").map_err(internal)?,
        occurred_at_ms: from_i64(row.try_get("occurred_at_ms").map_err(internal)?)?,
    })
}

fn validate_scope(organization_id: &str, workspace_id: &str) -> Result<(), JobError> {
    if safe_id(organization_id) && safe_id(workspace_id) {
        Ok(())
    } else {
        Err(JobError::InvalidInput)
    }
}

fn state_name(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Leased => "leased",
        JobState::Succeeded => "succeeded",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
    }
}

fn parse_state(value: &str) -> Result<JobState, JobError> {
    match value {
        "queued" => Ok(JobState::Queued),
        "leased" => Ok(JobState::Leased),
        "succeeded" => Ok(JobState::Succeeded),
        "failed" => Ok(JobState::Failed),
        "cancelled" => Ok(JobState::Cancelled),
        _ => Err(JobError::Internal),
    }
}

fn as_i64(value: u64) -> Result<i64, JobError> {
    i64::try_from(value).map_err(|_| JobError::InvalidInput)
}

fn from_i64(value: i64) -> Result<u64, JobError> {
    u64::try_from(value).map_err(|_| JobError::Internal)
}

fn from_i32(value: i32) -> Result<u32, JobError> {
    u32::try_from(value).map_err(|_| JobError::Internal)
}

fn internal<E>(_error: E) -> JobError {
    JobError::Internal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_forces_tenant_rls_and_contains_no_payload_column() {
        assert!(SCHEMA.contains("FORCE ROW LEVEL SECURITY"));
        assert!(SCHEMA.contains("current_setting('presto.organization_id', true)"));
        assert!(SCHEMA.contains("current_setting('presto.workspace_id', true)"));
        for forbidden in ["prompt", "document_body", "payload", "credential", "token"] {
            assert!(!SCHEMA.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn state_codec_is_strict() {
        assert_eq!(parse_state("queued"), Ok(JobState::Queued));
        assert_eq!(parse_state("unknown"), Err(JobError::Internal));
    }
}
