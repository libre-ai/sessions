**English** · [Français](README.fr.md)

> [!NOTE]
> **Reserved · future home of Sessions** — rebuilt in the canonical base repository [`libre-ai/libre-ai`](https://github.com/libre-ai/libre-ai) ([multi-repo topology, ADR-0008](https://github.com/libre-ai/libre-ai/blob/main/docs/adr/0008-multi-repo-target-topology-and-brand.md)).
> This repository will reopen as the real product repository when the owner activates it, consuming the base as a versioned dependency. The foundations described below are **being built now** — with links to the code that already exists.

# Sessions

**Source-grounded collective learning and facilitation.** Bring a group together around sourced materials — articles, evidence, expert input — with explicit roles (facilitator, participant, observer), audience rules for each contribution, and a **human approval gate** before any shared outcome is published. Never a silent synthesis; never an export that reveals private input by default.

The canonical brief it answers: _"How do we run a real-time, sourced collective learning session where every output is attributable and approval is mandatory?"_ — on data that participants own, in a space where evidence is cited, and where a facilitator can revoke sources or pause the session without losing history.

## Why it's different

- **Approval before publication.** Facilitators request a synthesis from bounded sources and participants' contributions; the output remains draft until a human explicitly approves it. Generation is helper, not authority.
- **Audience-scoped by default.** Every contribution carries an audience policy (public, shared with group, private). An export only includes content the requester is authorized to see; private input never leaks into shared outcomes silently.
- **Append-only and auditable.** All events — participants joining, contributions submitted, syntheses approved — are immutable and row-level-secured by organization. Revocation blocks future synthesis but never rewrites past evidence.
- **Sourced and bounded.** Synthesis can only reference sources explicitly attached and validated by the facilitator. RAG/retrieval is not authority; attached sources are.
- **Real-time, resilient collaboration.** Participants co-edit draft outcomes in real time when a self-hosted relay is available; the session degrades gracefully to append-only mode if the relay is unreachable, never losing data.
- **Fail-closed access.** An unknown participant, a missing role in the session, or a stale cursor is rejected outright. Never silently degraded.

## Status — spec-published, foundations under construction

Sessions is being built from a locked specification. It is **not released yet**; append-only persistence and authorization come first, and a good part of it already exists and is proven in the base repository:

| Foundation                                            | State                | Evidence                                                                                                                                                                                |
| ----------------------------------------------------- | -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Session event validator & append-only reducer**     | ✅ built             | Unit tested state transitions and idempotency ([#165](https://github.com/libre-ai/libre-ai/pull/165))                                                                                   |
| **Append-only event persistence with RLS**            | ✅ built, integrated | PostgreSQL row-level security, tenant isolation, cursor-based reconnect ([#173](https://github.com/libre-ai/libre-ai/pull/173))                                                         |
| **Authorization matrix & Biscuit policy**             | ✅ built, conformant | Membership roles, resource scope (session/contribution/outcome), revocation ([#174](https://github.com/libre-ai/libre-ai/pull/174))                                                     |
| **Command service & vertical composition**            | ✅ built             | Domain commands (CreateSession, JoinSession, SubmitContribution, ApproveOutcome), live ([#175](https://github.com/libre-ai/libre-ai/pull/175))                                          |
| **Accessible SSR cockpit — read view**                | ✅ built, HTTP ready | Keyboard navigation, session state read-through, event stream polling ([#179](https://github.com/libre-ai/libre-ai/pull/179))                                                           |
| **Real-time collaboration amendments — spec**         | ✅ spec-signed       | Owner-ratified collab design: CRDT + MLS E2EE, self-hosted relay, CollabCheckpointRecorded events, approval gate never weakened ([#198](https://github.com/libre-ai/libre-ai/pull/198)) |
| **Collaboration brick (CRDT + MLS) — implementation** | ⏳ next              | Sovereign end-to-end-encrypted co-editing; ciphertext-only relay; graceful degradation to append-only                                                                                   |
| **Command surface — write UI, export, deletion**      | ⏳ next              | Draft/approve workflows, audience-scoped export, retention and session closure                                                                                                          |
| **Generation & evidence adapter**                     | ⏳ next              | Bounded synthesis from sources/contributions, attenuated Biscuit for provider, draft failure handling                                                                                   |
| **Multi-instance & privacy qualification**            | ⏳ next              | Two-instance reconnect, private-export proof, cross-tenant denial, human approval journey                                                                                               |

This repository is a public reserved home; the legacy implementation it still carries is frozen for reference, and the rebuild happens in the base repository until activation (wave 4). **Benchmark target:** Miro — real-time collaborative facilitation tooling, reached through explicit approval and append-only events rather than real-time consensus.

## How it works

1. **Facilitate** — a facilitator creates a session, sets an audience policy for contributions (public, shared, private), and attaches validated sources (documents, expert responses, prior evidence).
2. **Participate** — participants join with scoped membership, contribute under the audience rules, and reconnect from a cursor without re-submitting contributions. Presence is ephemeral and cannot authorize.
3. **Synthesis** — the facilitator requests a synthesis from sources and contributions in scope; the output is drafted by a generation provider, but remains **draft only** until the facilitator explicitly approves it.
4. **Export & close** — an authorized actor exports an audience-specific bundle (only seeing content they can access), and the owner closes or deletes the session per retention contract. All events remain immutable.

## Architecture — built from interoperable bricks

Sessions is a product assembled from independently versioned bricks; each is usable and testable on its own, and the product is their composition (the multi-repo target of [ADR-0008](https://github.com/libre-ai/libre-ai/blob/main/docs/adr/0008-multi-repo-target-topology-and-brand.md)).

| Brick                                     | Role                                                | Interface it exposes / consumes                                                                                               |
| ----------------------------------------- | --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| **`sessions-core`** (TypeScript / Bun)    | Deterministic state machine and event reducer       | Domain commands, append-only event stream, audience projection, reconnect cursor logic                                        |
| **`@libre-ai/web-platform`**              | SSR / Bun BFF foundation                            | Request handler, WebSocket upgrade, server-side session evaluation, database query interface                                  |
| **`@libre-ai/data`**                      | Organization, session, and contribution persistence | PostgreSQL driver, row-level security policies, migration framework                                                           |
| **`collab-core`** (CRDT + MLS, Rust/WASM) | Real-time collaborative draft editing               | E2EE participant synchronization, ciphertext-only relay interface, CollabCheckpointRecorded event integration (⏳ planned)    |
| **Contracts**                             | Locked interoperability surface                     | `session-event.v1`, `session-export.v1`, `evidence-report.v1`, `sessions.v1.yaml` OpenAPI, `sessions-v1.datalog` authz policy |

The host (Bun server) holds the authorization token and evaluates commands against Biscuit policy; the event reducer runs deterministically local; real-time collab is capability-isolated (the relay receives only ciphertext, never cryptographic keys).

## Where the work happens

All active development is in the base repository, under:

- `apps/sessions` — the product host (SSR cockpit, event persistence, command service, UI)
- `src/domain` — state machine, event definitions, audience logic
- `src/persistence` — PostgreSQL RLS, reconnect, event cursor
- `src/authz` — Biscuit authorization policy, role validation
- `src/server` — WebSocket and HTTP command handlers
- `src/ui` — accessible read/write cockpit (React 19)
- `contracts/` — locked session event, export and API schemas
- [`docs/apps/sessions.md`](https://github.com/libre-ai/libre-ai/blob/main/docs/apps/sessions.md) — the full product specification

To follow progress or contribute, open issues and pull requests in [`libre-ai/libre-ai`](https://github.com/libre-ai/libre-ai). This repository stays reserved until activation.

## License

EUPL-1.2.
