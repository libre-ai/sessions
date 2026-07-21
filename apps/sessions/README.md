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

- Persistence (PostgreSQL, RLS, migrations), the WebSocket/reconnect transport,
  the HTTP API and Biscuit authorization, provider synthesis, audience
  projections and export manifests, and the participant UI.

## License

EUPL-1.2.
