# @libre-ai/sessions

Sessions supports sourced collective work: owners and facilitators control
membership and sources, participants contribute under explicit audience rules,
and human approval gates every shared outcome.

Work package: `WP-G3-S01`.

## Increment 5 — cockpit (accessible SSR read view)

`src/server/handler.ts` + `src/ui/sessions-cockpit.tsx` serve the read-only
sessions cockpit, server-rendered and usable **without JavaScript**. Per the spec's
runtime boundary the view is rendered from a contract fixture (`src/ui/fixture.ts`)
— no real session, transport or orchestrator integration until a bounded work
package and conformance review are approved.

- `createSessionsHandler` routes `/` to the SSR document and `/api/health` to a
  JSON status; an unknown route is `404`.
- `SessionsCockpit` renders an ordered, accessible table (a `<caption>`, `scope`
  column/row headers, a skip link and a `main` landmark). The session lifecycle is
  conveyed **as text, never colour**: the cumulative flags map to
  `Active → Close → Exportée → Supprimée` (the most-advanced terminal stage
  reached), with the revision and event count alongside.

Verified: the static render is a well-formed `<!doctype html>` document in French,
the table exposes its caption and header scopes, every fixture session is listed,
each lifecycle label renders, and no inline `style=` carries meaning; the handler
serves the cockpit, health, and a 404. Interactivity (command journeys, live
regions, audience projections) arrives in later increments.

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

## Increment 4 — command service (the composed vertical)

`src/app/execute-command.ts` composes the whole vertical for one action inside the
caller's tenant transaction. `executeSessionCommand(executor, principal, rawEvent,
recordedAt)` runs, fail-closed and cheapest-first:

1. `validateEvent` — structure (→ `sessions.cursor_invalid`);
2. `authorizeAction(role, type)` — the locked policy, **before any I/O** (→
   `sessions.membership_required`);
3. `loadSessionState` + `reduce` — ordering/identity/revision (→ `cursor_invalid`
   / `tenant_mismatch` / `revision_stale`);
4. `appendEvent` — persist; a concurrent writer taking the same sequence surfaces
   as `cursor_invalid` (the stream advanced past the append point).

It returns `accepted` (the event + the advanced state) or `refused` (the exact
code). Verified end-to-end against the real PostgreSQL barrier (PGlite): the
accepted path writes the log; an unauthorized action writes **nothing**; and the
structural, ordering, tenant and revision refusals each fire.

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

## RGPD — data-subject rights (rgpd-kit adoption)

Sessions is the first adopter of `@libre-ai/rgpd-kit`
(`packages/rgpd-kit/README.md`), entirely inside its bounded context: its own
tombstone/audit tables, its own deletion receipts, no cross-context table.

- **Port implementation** — `src/rgpd/data-subject-rights.ts` implements
  `DataSubjectRightsPort`: access (Art. 15), erasure (Art. 17), portability
  (Art. 20, `application/json` export) and restriction (Art. 18) fulfilled.
  `handleRestrictionRequest` is a refusal ladder, design-fixed order: a
  subject who is erased, unknown, or already restricted is refused
  (`subject_erased` / `subject_unknown` / `already_restricted`); otherwise the
  subject-supplied Art. 18(1) ground is recorded and the request is fulfilled
  with that ground and the count of affected records. All surfaces past
  verification speak opaque tenant-scoped sha-256 digests — never plaintext
  identifiers; subjects resolve through the indexed `actor_digest` column
  computed at append time (`0003_actor_digest.sql`).
- **The restriction invariant** — this is the binding read-path contract, not
  a description of wiring that exists today: any surface that discloses,
  transmits, derives from or destroys the subject's contributions MUST
  consult `isRestricted` (`src/rgpd/restriction.ts`) first — not just export
  and provider synthesis. That is why `isRestricted` is exported: v1 has no
  participant-facing transport that serves contributions to other
  participants, no export pipeline, and no synthesis/provider surface yet, so
  the invariant currently binds only their FUTURE implementations; it is
  consulted today only by the restriction flow itself. The ONE named
  exception, decided in the design, is the state fold (`loadSessionState` +
  the domain reducer): rebuilding `SessionState` from the event log is
  storage-integrity mechanics, it discloses nothing by itself, and carving it
  out is an explicit, reviewable stance rather than a silent gap.
- **What Art. 18(2) actually pauses** — restriction is neither erasure nor a
  read block on the subject: storage, the subject's own access/portability/
  erasure (Art. 15/17/20), and the subject's new contributions all stay
  allowed while restricted. What pauses is serving the subject's
  contributions to OTHER participants, exports, provider synthesis, and
  retention sweeps — every surface the invariant above binds.
- **The lift is two-step (Art. 18(3))** — `requestLift` opens the notice
  obligation: processing stays paused, and the audit trail records
  `sessions.rgpd.notice_required`. `confirmLift` completes the lift only once
  that obligation is discharged. v1's notice channel is an owner attestation
  recorded directly on the audit row (owner decision 2026-07-24) — no
  external notice channel exists yet. The two steps are joined by a synthetic
  `lift_<entry_seq>` id. The state store (`session_restricted_subjects`,
  `0004_restriction.sql`) is append-only — a lift or a re-restriction is
  always a new row, never an `UPDATE` — so the current state is always the
  row with the highest `entry_seq`.
- **Art. 19 recipients registry** — v1 records zero recipients: no export
  pipeline has ever run, so the Art. 19 duty to notify recipients of a
  restriction, erasure or rectification is vacuously satisfied today. The
  invariant BINDS the future export increment: it must record recipients per
  disclosure, so that a later restriction/erasure/rectification can trigger
  per-recipient notification, and the subject can ask for the list.
- **Cross-invariant with retention** — the rule the companion
  retention/compaction increment must implement and acceptance-test: a
  subject in `restricted` or `lift-pending` state is NEVER swept by the
  retention/compaction path (`packages/data`'s expired-record selection); the
  sweep's exclusion reads `session_restricted_subjects`. No sweep exists in
  `packages/data` yet, so nothing enforces this today — it binds that
  increment's future implementation. A restricted-then-erased subject remains
  compactable: the erasure request IS the subject's own Art. 18(2) consent to
  that one processing (deletion), so it supersedes the restriction on
  everything else.
- **Erasure semantics** — `session_events` is append-only, so Art. 17 removes
  **logical access in the accepted transaction**: `executeActiveDeletion`
  writes the `session_deleted_subjects` tombstone and the deletion receipt
  atomically; every RGPD read path refuses the subject from that row on.
  Physical compaction of the log follows the owner-scoped retention path
  (DATA-LIFECYCLE §Explicit deletion), which is why the declared categories
  carry `erasureScope: "deferred"`. **Receipt semantics:** the deletion
  receipt's `postgresql: deleted` outcome attests the accepted logical
  deletion of DATA-LIFECYCLE §Explicit deletion item 6 — access removed in
  the transaction — not physical row removal; `recordsAffected` counts rows
  made inaccessible. A platform follow-up (WP-G2-D01 surface) may let
  append-only stores carry a dedicated outcome/reason code in
  `executeActiveDeletion`.
- **Audit trail** — `session_subject_audit` (append-only, FORCE RLS) records
  `received` and the terminal `fulfilled`/`refused` state per request, plus
  the Art. 18(3) lift's `in-progress`/`fulfilled` notice pair; `detail`
  carries reason codes only (refusal codes and the lift-notice codes
  `sessions.rgpd.notice_required`/`notice_attested`), never free text.
- **Request handler** — `src/rgpd/request-handler.ts` is an exported factory,
  **not mounted** on the cockpit routes: the runtime boundary above stays
  locked until `WP-G3-S01`'s `sessions-authz-review` human gate. Authorization
  is deny-by-default against the locked operation matrix (access/portability
  need `export`, every other right needs `delete`) AND against the
  principal's own tenant — the body never chooses the tenant scope (K2).
  HTTP semantics: 403/404/400/405 cover authorization, verification and
  transport failures; a domain refusal (e.g. `already_erased`,
  `not_implemented`) is a first-class outcome returned as 200 with
  `meta.refusal` set.
- **Retention and consent** — content/outcomes follow the `sessions-content`
  rule (`contracts/data/retention.v1.json`: P90D, configurable P7D–P365D).
  Consent is the organizational-tenant membership model (implicit via
  membership, `legalBasis: "contract"`); per-purpose Art. 7(4) evaluation is
  available through rgpd-kit's consent module when Sessions needs finer
  grants.
- **Art. 30 declaration** — `art30-register.json`, contract-tested against
  the shared validator (`src/rgpd/art30-entry.test.ts`).

## License

EUPL-1.2.
