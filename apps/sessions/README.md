# @libre-ai/sessions

Sessions supports sourced collective work: owners and facilitators control
membership and sources, participants contribute under explicit audience rules,
and human approval gates every shared outcome.

Work package: `WP-G3-S01`.

## Increment 1 — session event validator and append-only reducer

`src/domain/session-event.ts` is the pure, offline heart of the session event
stream. It imports nothing, persists nothing, and transmits nothing.

| Function        | Guarantee                                                                                                                                  |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `validateEvent` | reconstructs a typed, contract-conformant `SessionEvent` from untrusted input; any structural/format violation → `sessions.cursor_invalid` |
| `reduce`        | folds a validated event onto the accumulated `SessionState`, enforcing the append-only invariants                                          |

Validation reuses the **locked `common.v1` definitions** the schema `$ref`s
(`urn`, `tenantId` = `^ten_[a-z0-9]{16,64}$`, `identifier` = `{2,127}`,
`revision` ≥ 0, `sha256`, `artifactReference`) — reused verbatim, not reinvented.
The `artifact` payload is typed as `ArtifactReference` (not an untyped escape
hatch), and the per-type conditional requirements (contribution → resource +
audience + digest; synthesis/outcome-approved → artifact) are enforced.

`reduce` enforces: a stream opens with `session-created` at sequence 1; sequence
is strictly contiguous; a single creation; revision never rewinds
(`sessions.revision_stale`); tenant identity is consistent
(`sessions.tenant_mismatch`); and nothing follows a deletion. `closed` /
`exported` / `deleted` are tracked on the state.

### Not yet built (deliberately deferred)

- The WebSocket/reconnect transport, the HTTP API and Biscuit authorization,
  provider synthesis, audience projections and export manifests, and the
  participant UI.

## Increment 3 — authorization matrix (sessions-v1 policy)

`src/authz/session-authorization.ts` mirrors the LOCKED authorizer policy
`contracts/authz/sessions-v1.datalog` in TypeScript, so the app refuses before
touching state (the datalog stays the source of truth — a conformance test parses
it and fails if the matrix drifts). Deny-by-default.

- `authorizeAction(role, eventType)` — producing a mutating event requires its
  operation; a role without it → `sessions.membership_required`. Owner may do
  everything; facilitator all but `add-member`/`delete`; participant only
  `join`/`submit`; observer no mutation.
- `authorizeRead(role, audience, isContributionOwner)` — owner/facilitator read
  any audience; a participant reads the `session` audience or a `private`
  contribution only when they own it; an observer reads only `session`; anything
  else → `sessions.audience_forbidden`.

Tenant scoping (`resource_tenant`) is enforced structurally by RLS and the
reducer, not here. Export-content rules (`private_export_forbidden`) and provider
attenuation stay deferred.

## Increment 2 — append-only event persistence (PostgreSQL / RLS)

`migrations/0001_sessions.sql` and `src/persistence/session-event-store.ts`
persist the session event stream in PostgreSQL behind the tenant barrier
(`packages/data`):

- **`session_events`** is an **append-only** table: a `tenant_id`-format `CHECK`,
  `FORCE` row-level security keyed on the `app.tenant_id` GUC, and a grant of
  `SELECT, INSERT` only — no `UPDATE`/`DELETE`, so the causal log is immutable even
  to the application role. The composite key `(tenant_id, session_id, sequence)`
  makes a replayed sequence conflict rather than fork.
- **`appendEvent`** inserts a validated event within the caller's tenant
  transaction, asserting the event's tenant matches the active context (defense in
  depth above RLS) and raising `SessionSequenceConflictError` on a replay.
- **`loadEvents`** / **`loadSessionState`** read a session's stream in causal order
  and rebuild state by folding it back through the domain reducer; a stream that
  does not reduce is surfaced as `SessionStreamCorruptError`, never silently used.

Verified against the real PostgreSQL barrier (PGlite): round-trip, reduction,
replay conflict, append-only grant (UPDATE/DELETE denied), and cross-tenant RLS
isolation.

## License

EUPL-1.2.
