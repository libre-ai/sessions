# ADR-0004 — Product-local jobs and sovereign AI provider routing

- Status: Accepted
- Date: 2026-07-11
- Authority: control-plane ADR 0038, 0041, 0043 and SES-025

## Context

Sessions needs resumable background work and grounded generation without turning Portal into a workflow engine or accepting arbitrary hosted AI endpoints. Provider URLs and one-process background tasks are security and reliability boundaries, not incidental configuration.

## Decision

The product owns `jobs::JobStore` and its state machine. The implementation provides an in-memory reference adapter and a PostgreSQL adapter with:

- organization/workspace-scoped idempotency;
- exclusive expiring leases and revision guards;
- heartbeats, recovery after lease expiry and bounded attempts;
- cooperative cancellation;
- metadata-only outbox events with bounded claims, expiring publisher leases and one-shot acknowledgement;
- PostgreSQL transactions, `FOR UPDATE SKIP LOCKED`, tenant settings and forced RLS on jobs and events;
- explicit schema application separated from runtime connection;
- no prompts, document bodies or credentials in records/events.

Portal receives only a projection of progress; it never owns leases, retries or product transitions. A live PostgreSQL conformance test is opt-in through `JOBS_DATABASE_URL`; production credentials must be non-superuser and must not hold `BYPASSRLS`. Static migration evidence is reproducible with:

```text
wrench-db-inspect run \
  --manifest docs/db/jobs-manifest.json \
  --schema-dump crates/server/migrations/0001_jobs_and_outbox.sql \
  --profile protected_branch
```

The static report does not replace the live role/RLS conformance test.

The AI transport keeps the OpenAI wire shape but closes routing:

- local development is loopback-only and explicitly enabled;
- hosted routing is Clever AI only, HTTPS and default-off;
- hosted construction requires a safe versioned `CLEVER_AI_CONTRACT_REF` plus explicit models and credential;
- direct Mistral/OpenAI/other hosted fallback is not configurable;
- redirects are disabled, timeout is 30 seconds, request bodies are bounded to 1 MiB and responses to 4 MiB;
- deterministic fakes remain the default test path.

No hosted call, account operation or provisioning is part of this decision.

## Consequences

A crashed worker can be recovered without duplicate concurrent ownership. Stale workers cannot complete a newer lease. Retry and cancellation are explicit observable transitions. Hosted provider activation remains impossible with legacy `AI_*` variables.

The PostgreSQL adapter remains pre-production until its ignored conformance test and Wrench RLS evidence run against the target PostgreSQL role. Before deployment, attach that evidence and the approved Clever AI contract reference without copying private contract content.

## Rollback

Disable all AI routes and use fixture-backed content. Stop leasing jobs; queued metadata remains inert. No provider-specific type exists in the domain contracts.
