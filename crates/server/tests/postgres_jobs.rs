//! PostgreSQL job/RLS/outbox conformance. No database is provisioned by this
//! test. Run explicitly against an expendable local database:
//!
//! `JOBS_DATABASE_URL=postgres://... cargo test --test postgres_jobs -- --ignored`

use std::time::{SystemTime, UNIX_EPOCH};

use presto_server::jobs::{EnqueueJob, JobStore};
use presto_server::postgres_jobs::PostgresJobStore;
use sqlx::postgres::PgPoolOptions;

fn unique_scope() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be after epoch")
        .as_nanos();
    format!("test_{nanos}")
}

#[tokio::test]
#[ignore = "requires explicit JOBS_DATABASE_URL; never provisions a database"]
async fn postgres_jobs_are_tenant_scoped_exclusive_and_publish_once() {
    let Ok(url) = std::env::var("JOBS_DATABASE_URL") else {
        eprintln!("skipping: set JOBS_DATABASE_URL to an expendable local PostgreSQL database");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    PostgresJobStore::apply_schema(&pool).await.unwrap();
    let store = PostgresJobStore::from_pool(pool);
    let organization_a = unique_scope();
    let organization_b = unique_scope();
    let workspace = "ws_test";
    let idempotency_key = format!("sha256:{}", "a".repeat(64));

    let enqueue = |organization_id: &str| EnqueueJob {
        organization_id: organization_id.to_string(),
        workspace_id: workspace.to_string(),
        kind: "synthetic_test".to_string(),
        idempotency_key: idempotency_key.clone(),
        max_attempts: 2,
    };
    let first = store.enqueue(enqueue(&organization_a)).await.unwrap();
    let duplicate = store.enqueue(enqueue(&organization_a)).await.unwrap();
    let other = store.enqueue(enqueue(&organization_b)).await.unwrap();
    assert_eq!(first.job_id, duplicate.job_id);
    assert_ne!(first.job_id, other.job_id);

    let (left, right) = tokio::join!(
        store.lease_next(&organization_a, workspace, "worker_a", 100, 1_000),
        store.lease_next(&organization_a, workspace, "worker_b", 100, 1_000),
    );
    let exclusive = [left.unwrap(), right.unwrap()]
        .into_iter()
        .filter(Option::is_some)
        .count();
    assert_eq!(exclusive, 1);

    let claims = store
        .claim_events(&organization_a, workspace, "publisher_a", 100, 1_000, 100)
        .await
        .unwrap();
    assert!(!claims.is_empty());
    assert!(
        claims
            .iter()
            .all(|claim| claim.event.organization_id == organization_a)
    );
    let claim = &claims[0];
    store
        .acknowledge_event(
            &organization_a,
            workspace,
            &claim.event.event_id,
            "publisher_a",
            &claim.claim_id,
            101,
        )
        .await
        .unwrap();
    let remaining = store
        .claim_events(&organization_a, workspace, "publisher_b", 102, 1_000, 100)
        .await
        .unwrap();
    assert!(
        remaining
            .iter()
            .all(|candidate| candidate.event.event_id != claim.event.event_id)
    );
}
