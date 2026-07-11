# ADR-0004 — Product-local jobs and sovereign AI provider routing

- Status: Accepted
- Date: 2026-07-11
- Authority: control-plane ADR 0038, 0041, 0043 and SES-025

## Context

Sessions needs resumable background work and grounded generation without turning Portal into a workflow engine or accepting arbitrary hosted AI endpoints. Provider URLs and one-process background tasks are security and reliability boundaries, not incidental configuration.

## Decision

The product owns `jobs::JobStore` and its state machine. The first implementation is an in-memory reference adapter with:

- organization/workspace-scoped idempotency;
- exclusive expiring leases and revision guards;
- heartbeats, recovery after lease expiry and bounded attempts;
- cooperative cancellation;
- metadata-only outbox events;
- no prompts, document bodies or credentials in records/events.

Portal receives only a projection of progress; it never owns leases, retries or product transitions. PostgreSQL persistence and RLS remain a separate adapter/gate.

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

The in-memory adapter is not production persistence. Before deployment, add PostgreSQL transactions/RLS, outbox claiming and Wrench evidence, then attach the approved Clever AI contract reference without copying private contract content.

## Rollback

Disable all AI routes and use fixture-backed content. Stop leasing jobs; queued metadata remains inert. No provider-specific type exists in the domain contracts.
