# rumble-lm — Design Corpus

The single map of the design documents and how they fit together. `rumble-lm` is
a **sovereign, self-hostable grounded notebook** (personal daily surface) with a
**live collaborative-quizzing** differentiator (NotebookLM × Kahoot), built in
Rust. This index keeps the specs coherent: one increment spine, shared
invariants, and an explicit cross-spec ledger.

## Document map

| Document                                                                                                                   | Concern                                                                           | Status    |
| -------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- | --------- |
| [`adr/0001-product-architecture-and-boundaries.md`](adr/0001-product-architecture-and-boundaries.md)                       | Product architecture, bricks (P1…P4, Client), the one-way dependency invariant    | Accepted  |
| [`adr/0002-mobile-first-webview-rust-core.md`](adr/0002-mobile-first-webview-rust-core.md)                                 | Mobile-first WebView/PWA path with Rust-core portability contract                 | Accepted  |
| [`adr/0003-companion-repositories.md`](adr/0003-companion-repositories.md)                                                   | Companion repos for adjacent sovereign tooling (ingestion, SQL audit, memory, supply-chain) | Accepted  |
| [`specs/2026-06-27-presto-matic-design.md`](specs/2026-06-27-presto-matic-design.md)                                       | The product design (wedge, differentiators, live protocol, sovereignty)           | Proposed  |
| [`specs/2026-06-28-collaborative-spaces-authz-design.md`](specs/2026-06-28-collaborative-spaces-authz-design.md)           | **SP-A** — authorization substrate (OIDC, spaces, membership, Biscuit caps)       | Proposed  |
| [`specs/2026-06-28-signed-classification-clearance-design.md`](specs/2026-06-28-signed-classification-clearance-design.md) | **SP-B** — signed classification (confidentiality / PII / integrity) + clearance  | Proposed  |
| [`specs/2026-06-28-frontend-dioxus-design-system-design.md`](specs/2026-06-28-frontend-dioxus-design-system-design.md)     | **SP-C** — Rust-first clients consuming Portal; local UI crate `rumble-lm-ui` replaced former `presto-ui` | Proposed  |
| [`evolution/2026-06-28-evolution-and-hardening-spec.md`](evolution/2026-06-28-evolution-and-hardening-spec.md)             | Cross-cutting critique + prioritized hardening roadmap (adversarially challenged) | Proposed  |
| [`plans/2026-06-27-p3-tracer-bullet.md`](plans/2026-06-27-p3-tracer-bullet.md)                                             | The live-session tracer-bullet plan (P3)                                          | Reference |
| [`deploy/clever-cloud.md`](deploy/clever-cloud.md)                                                                         | Sovereign deployment runbook (Clever Cloud)                                       | Reference |
| [`security/owner-web-auth.md`](security/owner-web-auth.md)                                                                 | Owner OIDC/session architecture, threat model and durability limits               | Implemented |
| [`security/rag-exact-evidence-gate.md`](security/rag-exact-evidence-gate.md)                                               | Exact-evidence lexical hardening and explicit anti-injection limits                | Defence in depth |
| [`security/approved-notebook-claims.md`](security/approved-notebook-claims.md)                                             | Immutable owner-notebook claim authority, HTTP boundary and precise proof limits   | Implemented MVP |
| [`security/owner-corpus.md`](security/owner-corpus.md)                                                                     | Bounded process-local owner uploads, exact-byte approval and retrieval limits       | Implemented MVP |

SP-A/B/C are written as a coherent family (SP-A is the substrate; SP-B classifies
over it; SP-C consumes both). The evolution spec critiques the **built** product
(P1–P3 + the four differentiators, already on `main`) and slots its hardening
work _alongside_ the spec roadmap below.

> **Implementation status (2026-06-29).** The `Status` column above is each
> _spec document's_ status, not the code's. The stable backend/RAG/live-session
> baseline includes **SP-A inc-1/2** (OIDC validation, space-scoped Biscuit tokens,
> anti-enumeration, MembershipStore + revocation recheck) and **SP-B inc-1/3**
> (signed integrity hashes, signed PII verdict, retrieval space/clearance scoping,
> live-generation gate), all **implemented and tested**, plus the full evolution
> gate-wall (cargo-deny + cargo-audit + guard-scan + coverage ≥ 80%). The §3
> AI-latency SLOs and **SP-C (front)** remain open. See
> [`status/2026-06-29-session-handoff.md`](status/2026-06-29-session-handoff.md)
> for the verified KPI-by-KPI status and the SP-C Increment 1 plan.

## The unified increment spine (risk-first, wedge-first)

The specs share one increment cadence. Each column is independently shippable and
green; nothing builds the rich collaborative layer before the wedge works.

|                                                 | **Increment 1 — Wedge core** (authenticated solo notebook + existing live)                                                                                                                                                                                                  | **Increment 2 — Collaboration** (shared spaces)                                                                                                                   | **Increment 3 — Rich** (governance, native, advanced)                                                                         |
| ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| **SP-A** (authz)                                | OIDC (Auth Code + PKCE); solo-space bootstrap; `session→space` generalization; `MembershipStore` (owner-only); token transport; 404 anti-enumeration; anonymous live join-links keep working                                                                                | durable membership + roles; revocation (short TTL + tightened recheck + fanout cache-invalidation); audit of sensitive actions; ownership transfer / orphan-owner | capability-links + bounded delegation; Keycloak directory (optional); quotas; corpus `space_id` scoping (coordinated with P1) |
| **SP-B** (classification)                       | signed **integrity** hash for the grounding wedge (solo; no access gating)                                                                                                                                                                                                  | manual **confidentiality** gating + retrieval filter + the invitation gate                                                                                        | automatic **PII** detection + hybrid clearance `min(org, grant)` + the **live-generation gate**                               |
| **SP-C** (front)                                | personal notebook (web/PWA): RAG chat, corpus view/upload, studio; core design system; SP-B confidentiality badges                                                                                                                                                          | `presto-join` guest/join + the live session UI (typed quiz, leaderboard, heatmap, breakouts)                                                                      | Tauri desktop + offline-local RAG; full design system + theming                                                               |
| **Evolution** (hardening, identity-independent) | **P0/P1** runs here: RAG prompt/lexical defence in depth (not a complete anti-injection proof); A1 reveal pure-function consolidation; Sec2 `deny.toml`+CI; O1 JSON logs; O2 verifier failure signal; Pf1 batch writes; P1a deep `/health`; Prod1 analytics API; GDPR1 session-data delete; Pf2 HNSW index + instrument | O3 multi-round + concurrent store tests; O4 multi-instance load test                                                                                              | measure-gated perf (embedding cache, async ingestion); persistent SRS (needs identity)                                        |

**Reading the spine:** Increment 1 is the only one needed for the wedge. The
evolution hardening is _not_ gated on identity — most of it (the security/quality
P0/P1) lands with Increment 1 and protects the product that already exists.

## Shared invariants (every spec honours these)

- **One-way dependency (ADR-0001).** `rag`/P1 and the front/Client **never depend on P4 (authz/classification) as code**. The `Retriever` (`corpus.rs`) _receives_ `space_id` (SP-A) and `max_confidentiality` (SP-B) as **opaque parameters** computed server-side; the ordered `Level` type lives in `presto-core` (transverse). Data flows in; there is no `rag → P4` code edge.
- **Server-authoritative.** The client renders the caps, state, and clearance it is granted; it never computes score, timing, rights, or clearance. Optimistic UI is allowed; truth comes from the server.
- **Token-is-not-a-cache.** The Biscuit encodes a capability minted _at access_ from OIDC identity + DB membership; Postgres stays the authority on _current_ membership (the basis of immediate revocation via the fanout-invalidated recheck cache).
- **Biscuit emitter discipline.** Sole emitter, Ed25519 key shared across instances, injected-clock minting, authorizer policies, `check if time < expiration` self-expiry, errors never carry the token. SP-A generalizes `session→space`; SP-B adds signed third-party blocks (classifier + ingestion keys, independent of the server's trust).
- **Token transport.** Web/PWA: `HttpOnly; Secure; SameSite=Strict` cookie + `Sec-Fetch-Site` check. Tauri: `Authorization` header + OS secure store. The wasm client never reads the token.
- **Sovereignty.** OSS licensing only (MIT/Apache/MPL family, enforced by the `deny.toml` + `cargo-audit` gates), EU residency, no US hyperscaler/gatekeeper, and no internal/proprietary-employer reference of any kind in any artifact (this is a clean-room sovereign OSS repo).
- **Portal client-platform boundary.** Shared tokens, accessibility, i18n UI, and web/native adapters belong to Portal. `presto-ui` was the legacy local crate name; the crate is now `rumble-lm-ui` for LM-specific components.
- **Companion repos, not hidden runtime deps.** ADR-0003 splits adjacent capabilities into `gear-loader`, `gear-memory`, `wrench-db-inspect`, `gear-depot`, `gear-cable`, and Portal client-platform repos. rumble-lm integrates through stable contracts (HTTP/queue/object-store/CLI artifacts), never by depending on companion internals.

## Cross-spec coherence ledger (open items spanning specs)

Tracked here because no single spec owns them:

1. **Session-engine multi-instance consistency** — unowned by SP-A/B/C (they cover the _auth_ layer's multi-instance story). The session-engine race (two instances creating one session id; a `reveal` racing an in-flight answer across instances) needs a **dedicated ADR**; the evolution `concurrent` test (O3) is the first probe. _(evolution §8.4)_
2. **The live-generation path carries distinct controls.** Prompt fences and exact lexical evidence are identity-independent defence in depth, not an anti-injection authority (evolution §3.1). SP-B's live-generation gate controls source confidentiality (audience ceiling, inc-3). Any security-sensitive `Grounded` projection additionally needs independent server-side approved claims; a signed source-integrity hash does not make poisoned content trustworthy.
3. **Erasure vs grounding** — SP-B open item: a cited-then-erased source — does dependent generated content get invalidated? Coordinate SP-B erasure + SP-A audit + the studio (P2).
4. **SP-A refinements surfaced by the evolution challenge** (open-item level, not a redesign): (a) state that audit `actor_sub` is pseudonymous and retained under the GDPR Art. 17(3) legal-obligation exception (closes the audit-vs-erasure contradiction explicitly); (b) specify _where_ `RateLimited`/429 applies (OIDC callback, capability-link single-use redemption, login) and connect it to the already-shipped global rate-limit (`ratelimit.rs`, P12).
5. **Corpus columns are one coordinated change.** `space_id` (SP-A inc-3) and `confidentiality` (SP-B inc-2/3) both land in `corpus.rs` (P1), coordinated with ingestion (P11). The `Retriever` receives both as opaque params in a single migration — not two separate authz-driven edits.
