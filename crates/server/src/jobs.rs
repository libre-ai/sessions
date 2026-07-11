//! Product-local leased jobs and metadata-only outbox.
//!
//! Portal may project these records for UI, but does not own this lifecycle.
//! Payloads, prompts and document contents are deliberately absent.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const MAX_JOB_LEASE_MS: u64 = 15 * 60 * 1_000;
pub(crate) const MAX_OUTBOX_LEASE_MS: u64 = 5 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Leased,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub organization_id: String,
    pub workspace_id: String,
    pub job_id: String,
    pub kind: String,
    pub idempotency_key: String,
    pub state: JobState,
    pub revision: u64,
    pub attempts: u32,
    pub max_attempts: u32,
    pub lease_owner: Option<String>,
    pub lease_expires_at_ms: Option<u64>,
    pub cancel_requested: bool,
    pub result_ref: Option<String>,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueJob {
    pub organization_id: String,
    pub workspace_id: String,
    pub kind: String,
    pub idempotency_key: String,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseGuard {
    pub organization_id: String,
    pub workspace_id: String,
    pub job_id: String,
    pub worker_id: String,
    pub expected_revision: u64,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    pub lease: LeaseGuard,
    pub extend_by_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteJob {
    pub lease: LeaseGuard,
    pub completion: Completion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    Succeeded {
        result_ref: String,
    },
    Failed {
        failure_code: String,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobEvent {
    pub event_id: String,
    pub organization_id: String,
    pub workspace_id: String,
    pub job_id: String,
    pub revision: u64,
    pub event_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxClaim {
    pub event: JobEvent,
    pub publisher_id: String,
    pub claim_id: String,
    pub claim_expires_at_ms: u64,
    pub delivery_attempt: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    InvalidInput,
    NotFound,
    WrongState,
    LeaseOwnerMismatch,
    LeaseExpired,
    StaleRevision,
    OutboxClaimMismatch,
    Internal,
}

impl std::fmt::Display for JobError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput => "job input is invalid",
            Self::NotFound => "job was not found in tenant scope",
            Self::WrongState => "job state does not allow this transition",
            Self::LeaseOwnerMismatch => "job lease belongs to another worker",
            Self::LeaseExpired => "job lease has expired",
            Self::StaleRevision => "job revision is stale",
            Self::OutboxClaimMismatch => "outbox claim is stale or belongs to another publisher",
            Self::Internal => "job store operation failed",
        })
    }
}

impl std::error::Error for JobError {}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn enqueue(&self, request: EnqueueJob) -> Result<JobRecord, JobError>;
    async fn lease_next(
        &self,
        organization_id: &str,
        workspace_id: &str,
        worker_id: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<Option<JobRecord>, JobError>;
    async fn heartbeat(&self, heartbeat: Heartbeat) -> Result<JobRecord, JobError>;
    async fn request_cancel(
        &self,
        organization_id: &str,
        workspace_id: &str,
        job_id: &str,
    ) -> Result<JobRecord, JobError>;
    async fn complete(&self, request: CompleteJob) -> Result<JobRecord, JobError>;
    async fn events(
        &self,
        organization_id: &str,
        workspace_id: &str,
        limit: u32,
    ) -> Result<Vec<JobEvent>, JobError>;
    async fn claim_events(
        &self,
        organization_id: &str,
        workspace_id: &str,
        publisher_id: &str,
        now_ms: u64,
        lease_ms: u64,
        limit: u32,
    ) -> Result<Vec<OutboxClaim>, JobError>;
    async fn acknowledge_event(
        &self,
        organization_id: &str,
        workspace_id: &str,
        event_id: &str,
        publisher_id: &str,
        claim_id: &str,
        now_ms: u64,
    ) -> Result<(), JobError>;
}

#[derive(Default)]
pub struct InMemoryJobStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<TenantJobKey, JobRecord>,
    idempotency: HashMap<IdempotencyKey, String>,
    events: Vec<OutboxEntry>,
    job_sequences: HashMap<TenantJobKey, u64>,
    next_job_sequence: u64,
}

struct OutboxEntry {
    event: JobEvent,
    claim: Option<OutboxClaimState>,
    published: bool,
    delivery_attempts: u32,
}

struct OutboxClaimState {
    publisher_id: String,
    claim_id: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TenantJobKey {
    organization_id: String,
    workspace_id: String,
    job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IdempotencyKey {
    organization_id: String,
    workspace_id: String,
    value: String,
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl JobStore for InMemoryJobStore {
    async fn enqueue(&self, request: EnqueueJob) -> Result<JobRecord, JobError> {
        validate_enqueue(&request)?;
        let mut inner = self.inner.lock();
        let idempotency = IdempotencyKey {
            organization_id: request.organization_id.clone(),
            workspace_id: request.workspace_id.clone(),
            value: request.idempotency_key.clone(),
        };
        if let Some(job_id) = inner.idempotency.get(&idempotency) {
            return inner
                .jobs
                .get(&tenant_key(
                    &request.organization_id,
                    &request.workspace_id,
                    job_id,
                ))
                .cloned()
                .ok_or(JobError::Internal);
        }

        let record = JobRecord {
            organization_id: request.organization_id,
            workspace_id: request.workspace_id,
            job_id: format!("job_{}", Uuid::new_v4().simple()),
            kind: request.kind,
            idempotency_key: request.idempotency_key,
            state: JobState::Queued,
            revision: 1,
            attempts: 0,
            max_attempts: request.max_attempts,
            lease_owner: None,
            lease_expires_at_ms: None,
            cancel_requested: false,
            result_ref: None,
            failure_code: None,
        };
        inner.idempotency.insert(idempotency, record.job_id.clone());
        let key = tenant_key(
            &record.organization_id,
            &record.workspace_id,
            &record.job_id,
        );
        let sequence = inner.next_job_sequence;
        inner.next_job_sequence = inner.next_job_sequence.saturating_add(1);
        inner.job_sequences.insert(key, sequence);
        insert_with_event(&mut inner, record.clone(), "job_queued");
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
        if !safe_id(organization_id)
            || !safe_id(workspace_id)
            || !safe_id(worker_id)
            || !safe_timestamp(now_ms)
            || !(1..=MAX_JOB_LEASE_MS).contains(&lease_ms)
        {
            return Err(JobError::InvalidInput);
        }
        let lease_expires_at_ms = now_ms.checked_add(lease_ms).ok_or(JobError::InvalidInput)?;
        let mut inner = self.inner.lock();
        let exhausted: Vec<_> = inner
            .jobs
            .iter()
            .filter(|(key, job)| {
                key.organization_id == organization_id
                    && key.workspace_id == workspace_id
                    && job.state == JobState::Leased
                    && job
                        .lease_expires_at_ms
                        .is_some_and(|expiry| expiry <= now_ms)
                    && job.attempts >= job.max_attempts
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in exhausted {
            let job = inner.jobs.get_mut(&key).ok_or(JobError::Internal)?;
            job.state = JobState::Failed;
            job.revision += 1;
            job.lease_owner = None;
            job.lease_expires_at_ms = None;
            job.failure_code = Some("lease_attempts_exhausted".to_string());
            let record = job.clone();
            append_event(&mut inner, &record, "job_failed");
        }
        let mut candidates: Vec<_> = inner
            .jobs
            .iter()
            .filter(|(key, job)| {
                key.organization_id == organization_id
                    && key.workspace_id == workspace_id
                    && job.attempts < job.max_attempts
                    && (job.state == JobState::Queued
                        || job.state == JobState::Leased
                            && job
                                .lease_expires_at_ms
                                .is_some_and(|expiry| expiry <= now_ms))
            })
            .map(|(key, job)| {
                (
                    key.clone(),
                    job.revision,
                    inner.job_sequences.get(key).copied().unwrap_or(u64::MAX),
                )
            })
            .collect();
        candidates.sort_by(|left, right| {
            left.2
                .cmp(&right.2)
                .then_with(|| left.0.job_id.cmp(&right.0.job_id))
        });
        let Some((key, expected_revision, _)) = candidates.first().cloned() else {
            return Ok(None);
        };
        let job = inner.jobs.get_mut(&key).ok_or(JobError::Internal)?;
        if job.revision != expected_revision {
            return Err(JobError::StaleRevision);
        }
        job.state = JobState::Leased;
        job.revision += 1;
        job.attempts = job.attempts.saturating_add(1);
        job.lease_owner = Some(worker_id.to_string());
        job.lease_expires_at_ms = Some(lease_expires_at_ms);
        job.failure_code = None;
        let record = job.clone();
        append_event(&mut inner, &record, "job_leased");
        Ok(Some(record))
    }

    async fn heartbeat(&self, heartbeat: Heartbeat) -> Result<JobRecord, JobError> {
        validate_lease_guard(&heartbeat.lease)?;
        if !(1..=MAX_JOB_LEASE_MS).contains(&heartbeat.extend_by_ms) {
            return Err(JobError::InvalidInput);
        }
        let lease_expires_at_ms = heartbeat
            .lease
            .now_ms
            .checked_add(heartbeat.extend_by_ms)
            .ok_or(JobError::InvalidInput)?;
        let mut inner = self.inner.lock();
        let key = tenant_key(
            &heartbeat.lease.organization_id,
            &heartbeat.lease.workspace_id,
            &heartbeat.lease.job_id,
        );
        let job = inner.jobs.get_mut(&key).ok_or(JobError::NotFound)?;
        verify_lease(
            job,
            &heartbeat.lease.worker_id,
            heartbeat.lease.expected_revision,
            heartbeat.lease.now_ms,
        )?;
        job.revision += 1;
        job.lease_expires_at_ms = Some(lease_expires_at_ms);
        let record = job.clone();
        append_event(&mut inner, &record, "job_heartbeat");
        Ok(record)
    }

    async fn request_cancel(
        &self,
        organization_id: &str,
        workspace_id: &str,
        job_id: &str,
    ) -> Result<JobRecord, JobError> {
        if !safe_id(organization_id) || !safe_id(workspace_id) || !safe_id(job_id) {
            return Err(JobError::InvalidInput);
        }
        let mut inner = self.inner.lock();
        let key = tenant_key(organization_id, workspace_id, job_id);
        let job = inner.jobs.get_mut(&key).ok_or(JobError::NotFound)?;
        match job.state {
            JobState::Queued => {
                job.state = JobState::Cancelled;
                job.revision += 1;
            }
            JobState::Leased => {
                job.cancel_requested = true;
                job.revision += 1;
            }
            JobState::Succeeded | JobState::Failed | JobState::Cancelled => {
                return Err(JobError::WrongState);
            }
        }
        let record = job.clone();
        append_event(&mut inner, &record, "job_cancel_requested");
        Ok(record)
    }

    async fn complete(&self, request: CompleteJob) -> Result<JobRecord, JobError> {
        validate_lease_guard(&request.lease)?;
        validate_completion(&request.completion)?;
        let mut inner = self.inner.lock();
        let key = tenant_key(
            &request.lease.organization_id,
            &request.lease.workspace_id,
            &request.lease.job_id,
        );
        let job = inner.jobs.get_mut(&key).ok_or(JobError::NotFound)?;
        verify_lease(
            job,
            &request.lease.worker_id,
            request.lease.expected_revision,
            request.lease.now_ms,
        )?;
        job.revision += 1;
        job.lease_owner = None;
        job.lease_expires_at_ms = None;
        let event_type = if job.cancel_requested {
            job.state = JobState::Cancelled;
            job.result_ref = None;
            job.failure_code = None;
            "job_cancelled"
        } else {
            match request.completion {
                Completion::Succeeded { result_ref } => {
                    job.state = JobState::Succeeded;
                    job.result_ref = Some(result_ref);
                    job.failure_code = None;
                    "job_succeeded"
                }
                Completion::Failed {
                    failure_code,
                    retryable,
                } => {
                    job.failure_code = Some(failure_code);
                    if retryable && job.attempts < job.max_attempts {
                        job.state = JobState::Queued;
                        "job_retry_queued"
                    } else {
                        job.state = JobState::Failed;
                        "job_failed"
                    }
                }
            }
        };
        let record = job.clone();
        append_event(&mut inner, &record, event_type);
        Ok(record)
    }

    async fn events(
        &self,
        organization_id: &str,
        workspace_id: &str,
        limit: u32,
    ) -> Result<Vec<JobEvent>, JobError> {
        if !safe_id(organization_id) || !safe_id(workspace_id) || !(1..=1_000).contains(&limit) {
            return Err(JobError::InvalidInput);
        }
        let mut events = self
            .inner
            .lock()
            .events
            .iter()
            .rev()
            .filter(|entry| {
                entry.event.organization_id == organization_id
                    && entry.event.workspace_id == workspace_id
            })
            .take(limit as usize)
            .map(|entry| entry.event.clone())
            .collect::<Vec<_>>();
        events.reverse();
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
        if !safe_id(organization_id)
            || !safe_id(workspace_id)
            || !safe_id(publisher_id)
            || !safe_timestamp(now_ms)
            || !(1..=MAX_OUTBOX_LEASE_MS).contains(&lease_ms)
            || !(1..=100).contains(&limit)
        {
            return Err(JobError::InvalidInput);
        }
        let claim_expires_at_ms = now_ms.checked_add(lease_ms).ok_or(JobError::InvalidInput)?;
        let mut inner = self.inner.lock();
        let mut claimed = Vec::new();
        for entry in inner.events.iter_mut().filter(|entry| {
            !entry.published
                && entry.event.organization_id == organization_id
                && entry.event.workspace_id == workspace_id
                && entry
                    .claim
                    .as_ref()
                    .is_none_or(|claim| claim.expires_at_ms <= now_ms)
        }) {
            if claimed.len() >= limit as usize {
                break;
            }
            entry.delivery_attempts = entry.delivery_attempts.saturating_add(1);
            let claim_id = format!("claim_{}", Uuid::new_v4().simple());
            entry.claim = Some(OutboxClaimState {
                publisher_id: publisher_id.to_string(),
                claim_id: claim_id.clone(),
                expires_at_ms: claim_expires_at_ms,
            });
            claimed.push(OutboxClaim {
                event: entry.event.clone(),
                publisher_id: publisher_id.to_string(),
                claim_id,
                claim_expires_at_ms,
                delivery_attempt: entry.delivery_attempts,
            });
        }
        Ok(claimed)
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
        if !safe_id(organization_id)
            || !safe_id(workspace_id)
            || !safe_id(event_id)
            || !safe_id(publisher_id)
            || !safe_id(claim_id)
            || !safe_timestamp(now_ms)
        {
            return Err(JobError::InvalidInput);
        }
        let mut inner = self.inner.lock();
        let entry = inner
            .events
            .iter_mut()
            .find(|entry| {
                entry.event.organization_id == organization_id
                    && entry.event.workspace_id == workspace_id
                    && entry.event.event_id == event_id
            })
            .ok_or(JobError::NotFound)?;
        let Some(claim) = &entry.claim else {
            return Err(JobError::OutboxClaimMismatch);
        };
        if entry.published
            || claim.publisher_id != publisher_id
            || claim.claim_id != claim_id
            || claim.expires_at_ms <= now_ms
        {
            return Err(JobError::OutboxClaimMismatch);
        }
        entry.published = true;
        entry.claim = None;
        Ok(())
    }
}

pub(crate) fn validate_completion(completion: &Completion) -> Result<(), JobError> {
    let valid = match completion {
        Completion::Succeeded { result_ref } => safe_reference(result_ref),
        Completion::Failed { failure_code, .. } => safe_code(failure_code),
    };
    if valid {
        Ok(())
    } else {
        Err(JobError::InvalidInput)
    }
}

pub(crate) fn validate_lease_guard(lease: &LeaseGuard) -> Result<(), JobError> {
    if safe_id(&lease.organization_id)
        && safe_id(&lease.workspace_id)
        && safe_id(&lease.job_id)
        && safe_id(&lease.worker_id)
        && safe_timestamp(lease.now_ms)
    {
        Ok(())
    } else {
        Err(JobError::InvalidInput)
    }
}

pub(crate) fn validate_enqueue(request: &EnqueueJob) -> Result<(), JobError> {
    if !safe_id(&request.organization_id)
        || !safe_id(&request.workspace_id)
        || !safe_code(&request.kind)
        || !safe_reference(&request.idempotency_key)
        || !(1..=10).contains(&request.max_attempts)
    {
        return Err(JobError::InvalidInput);
    }
    Ok(())
}

pub(crate) fn verify_lease(
    job: &JobRecord,
    worker_id: &str,
    expected_revision: u64,
    now_ms: u64,
) -> Result<(), JobError> {
    if job.state != JobState::Leased {
        return Err(JobError::WrongState);
    }
    if job.revision != expected_revision {
        return Err(JobError::StaleRevision);
    }
    if job.lease_owner.as_deref() != Some(worker_id) {
        return Err(JobError::LeaseOwnerMismatch);
    }
    if job
        .lease_expires_at_ms
        .is_none_or(|expiry| expiry <= now_ms)
    {
        return Err(JobError::LeaseExpired);
    }
    Ok(())
}

fn tenant_key(organization_id: &str, workspace_id: &str, job_id: &str) -> TenantJobKey {
    TenantJobKey {
        organization_id: organization_id.to_string(),
        workspace_id: workspace_id.to_string(),
        job_id: job_id.to_string(),
    }
}

fn insert_with_event(inner: &mut Inner, record: JobRecord, event_type: &str) {
    append_event(inner, &record, event_type);
    inner.jobs.insert(
        tenant_key(
            &record.organization_id,
            &record.workspace_id,
            &record.job_id,
        ),
        record,
    );
}

fn append_event(inner: &mut Inner, record: &JobRecord, event_type: &str) {
    inner.events.push(OutboxEntry {
        event: JobEvent {
            event_id: format!("evt_{}", Uuid::new_v4().simple()),
            organization_id: record.organization_id.clone(),
            workspace_id: record.workspace_id.clone(),
            job_id: record.job_id.clone(),
            revision: record.revision,
            event_type: event_type.to_string(),
        },
        claim: None,
        published: false,
        delivery_attempts: 0,
    });
}

pub(crate) fn safe_timestamp(value: u64) -> bool {
    value <= i64::MAX as u64
}

pub(crate) fn safe_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn safe_code(value: &str) -> bool {
    (1..=96).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn safe_reference(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'/' | b'.')
        })
        && !value.contains("..")
        && !value.contains("://")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> EnqueueJob {
        EnqueueJob {
            organization_id: "org_a".to_string(),
            workspace_id: "ws_a".to_string(),
            kind: "source_ingestion".to_string(),
            idempotency_key: "sha256:synthetic-idempotency".to_string(),
            max_attempts: 2,
        }
    }

    fn guard(job_id: &str, worker_id: &str, revision: u64, now_ms: u64) -> LeaseGuard {
        LeaseGuard {
            organization_id: "org_a".to_string(),
            workspace_id: "ws_a".to_string(),
            job_id: job_id.to_string(),
            worker_id: worker_id.to_string(),
            expected_revision: revision,
            now_ms,
        }
    }

    #[tokio::test]
    async fn enqueue_is_tenant_scoped_and_idempotent() {
        let store = InMemoryJobStore::new();
        let first = store.enqueue(request()).await.unwrap();
        let duplicate = store.enqueue(request()).await.unwrap();
        assert_eq!(first.job_id, duplicate.job_id);

        let mut other_tenant = request();
        other_tenant.organization_id = "org_b".to_string();
        let other = store.enqueue(other_tenant).await.unwrap();
        assert_ne!(first.job_id, other.job_id);
        assert!(
            store
                .events("org_b", "ws_a", 100)
                .await
                .unwrap()
                .iter()
                .all(|event| event.organization_id == "org_b")
        );
    }

    #[tokio::test]
    async fn lease_order_is_fifo_and_event_reads_are_bounded() {
        let store = InMemoryJobStore::new();
        let first = store.enqueue(request()).await.unwrap();
        let mut second_request = request();
        second_request.idempotency_key = "sha256:second".to_string();
        store.enqueue(second_request).await.unwrap();
        let leased = store
            .lease_next("org_a", "ws_a", "worker_a", 100, 20)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(leased.job_id, first.job_id);
        assert_eq!(store.events("org_a", "ws_a", 1).await.unwrap().len(), 1);
        assert_eq!(
            store.events("org_a", "ws_a", 0).await,
            Err(JobError::InvalidInput)
        );
    }

    #[tokio::test]
    async fn expired_lease_is_recovered_and_stale_worker_is_rejected() {
        let store = InMemoryJobStore::new();
        let job = store.enqueue(request()).await.unwrap();
        let first = store
            .lease_next("org_a", "ws_a", "worker_a", 100, 10)
            .await
            .unwrap()
            .unwrap();
        let recovered = store
            .lease_next("org_a", "ws_a", "worker_b", 111, 10)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.job_id, job.job_id);
        assert_eq!(recovered.attempts, 2);
        assert_eq!(
            store
                .complete(CompleteJob {
                    lease: guard(&job.job_id, "worker_a", first.revision, 105),
                    completion: Completion::Succeeded {
                        result_ref: "artifact:stale".to_string(),
                    },
                })
                .await,
            Err(JobError::StaleRevision)
        );
    }

    #[tokio::test]
    async fn repeated_lease_expiry_exhausts_attempt_budget() {
        let store = InMemoryJobStore::new();
        let job = store.enqueue(request()).await.unwrap();
        store
            .lease_next("org_a", "ws_a", "worker_a", 100, 10)
            .await
            .unwrap()
            .unwrap();
        store
            .lease_next("org_a", "ws_a", "worker_b", 111, 10)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store
                .lease_next("org_a", "ws_a", "worker_c", 122, 10)
                .await
                .unwrap(),
            None
        );
        let events = store.events("org_a", "ws_a", 100).await.unwrap();
        assert!(
            events
                .iter()
                .any(|event| { event.job_id == job.job_id && event.event_type == "job_failed" })
        );
    }

    #[tokio::test]
    async fn heartbeat_cancel_and_completion_are_revision_guarded() {
        let store = InMemoryJobStore::new();
        let job = store.enqueue(request()).await.unwrap();
        let leased = store
            .lease_next("org_a", "ws_a", "worker_a", 100, 20)
            .await
            .unwrap()
            .unwrap();
        let heartbeat = store
            .heartbeat(Heartbeat {
                lease: guard(&job.job_id, "worker_a", leased.revision, 110),
                extend_by_ms: 20,
            })
            .await
            .unwrap();
        let cancelling = store
            .request_cancel("org_a", "ws_a", &job.job_id)
            .await
            .unwrap();
        assert!(cancelling.cancel_requested);
        let cancelled = store
            .complete(CompleteJob {
                lease: guard(&job.job_id, "worker_a", cancelling.revision, 115),
                completion: Completion::Succeeded {
                    result_ref: "artifact:ignored".to_string(),
                },
            })
            .await
            .unwrap();
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(heartbeat.lease_owner.as_deref(), Some("worker_a"));
    }

    #[tokio::test]
    async fn invalid_completion_does_not_mutate_the_lease() {
        let store = InMemoryJobStore::new();
        let job = store.enqueue(request()).await.unwrap();
        let leased = store
            .lease_next("org_a", "ws_a", "worker_a", 100, 20)
            .await
            .unwrap()
            .unwrap();
        let invalid = store
            .complete(CompleteJob {
                lease: guard(&job.job_id, "worker_a", leased.revision, 110),
                completion: Completion::Succeeded {
                    result_ref: "https://forbidden.example/result".to_string(),
                },
            })
            .await;
        assert_eq!(invalid, Err(JobError::InvalidInput));
        let completed = store
            .complete(CompleteJob {
                lease: guard(&job.job_id, "worker_a", leased.revision, 110),
                completion: Completion::Succeeded {
                    result_ref: "artifact:valid".to_string(),
                },
            })
            .await
            .unwrap();
        assert_eq!(completed.state, JobState::Succeeded);
    }

    #[tokio::test]
    async fn outbox_claims_are_tenant_scoped_recoverable_and_one_shot() {
        let store = InMemoryJobStore::new();
        store.enqueue(request()).await.unwrap();
        let mut other_tenant = request();
        other_tenant.organization_id = "org_b".to_string();
        store.enqueue(other_tenant).await.unwrap();

        let first = store
            .claim_events("org_a", "ws_a", "publisher_a", 100, 10, 10)
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].delivery_attempt, 1);
        assert!(
            store
                .claim_events("org_b", "ws_a", "publisher_a", 100, 10, 10)
                .await
                .unwrap()
                .iter()
                .all(|claim| claim.event.organization_id == "org_b")
        );
        assert!(
            store
                .claim_events("org_a", "ws_a", "publisher_b", 105, 10, 10)
                .await
                .unwrap()
                .is_empty()
        );

        let recovered = store
            .claim_events("org_a", "ws_a", "publisher_b", 111, 10, 10)
            .await
            .unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].delivery_attempt, 2);
        assert_ne!(first[0].claim_id, recovered[0].claim_id);
        assert_eq!(
            store
                .acknowledge_event(
                    "org_a",
                    "ws_a",
                    &first[0].event.event_id,
                    "publisher_a",
                    &first[0].claim_id,
                    112,
                )
                .await,
            Err(JobError::OutboxClaimMismatch)
        );
        store
            .acknowledge_event(
                "org_a",
                "ws_a",
                &recovered[0].event.event_id,
                "publisher_b",
                &recovered[0].claim_id,
                112,
            )
            .await
            .unwrap();
        assert!(
            store
                .claim_events("org_a", "ws_a", "publisher_c", 113, 10, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn retry_is_bounded_by_max_attempts() {
        let store = InMemoryJobStore::new();
        let job = store.enqueue(request()).await.unwrap();
        let first = store
            .lease_next("org_a", "ws_a", "worker_a", 100, 20)
            .await
            .unwrap()
            .unwrap();
        let queued = store
            .complete(CompleteJob {
                lease: guard(&job.job_id, "worker_a", first.revision, 110),
                completion: Completion::Failed {
                    failure_code: "provider_unavailable".to_string(),
                    retryable: true,
                },
            })
            .await
            .unwrap();
        assert_eq!(queued.state, JobState::Queued);
        let second = store
            .lease_next("org_a", "ws_a", "worker_b", 120, 20)
            .await
            .unwrap()
            .unwrap();
        let failed = store
            .complete(CompleteJob {
                lease: guard(&job.job_id, "worker_b", second.revision, 125),
                completion: Completion::Failed {
                    failure_code: "provider_unavailable".to_string(),
                    retryable: true,
                },
            })
            .await
            .unwrap();
        assert_eq!(failed.state, JobState::Failed);
    }
}
