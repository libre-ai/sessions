# Session handoff — 2026-06-29

Stable-baseline snapshot for the next session. **Reverify before trusting it**
(it is a point-in-time snapshot, not a source of truth):

```bash
git -C "$(git rev-parse --show-toplevel)" log --oneline -8
git status --short
cargo test --workspace            # in-memory suite
gh run list --branch main --workflow ci --limit 1   # post-merge CI conclusion
```

Source branch: **`goal/moat-and-gatewall`** — verified CI-green (4 jobs) before
stable merge. After merge/tag, `main` and the stable tag are the source of truth.

---

## Delivered this session — the goal's §1 moat + §2 gate-wall + measurable §3

The goal (`docs/goals/2026-06-28-session-spec-coverage.md`) went from "§1 mostly
blocked" to **§1 complete**. Each KPI has a named source test, green in CI
(AI-gated tests self-skip in CI and are proven locally — see §3).

### §1 moat — 19/19 KPIs proven

| Area               | KPI                                         | Source test                                                                         |
| ------------------ | ------------------------------------------- | ----------------------------------------------------------------------------------- |
| Auth               | OIDC reject iss/aud/exp/nonce/sig           | `server::oidc` (7 adversarial tests)                                                |
|                    | Token space A ≠ B + capability enforcement  | `server::auth::space_token_isolates_spaces_and_enforces_caps`                       |
|                    | Anonymous live join-link still works        | `server/tests/ws_integration.rs::late_joiner_*` (session path untouched)            |
|                    | Anti-enumeration (404 identical)            | `server::authz::denials_are_indistinguishable_anti_enumeration`                     |
|                    | Revocation despite valid token              | `server::membership::revoked_member_is_denied_a_sensitive_op_despite_a_valid_token` |
| Ingestion          | Instruction-like payload exercised (not an anti-injection proof) | `server/tests/live_rag.rs` (real model, local)                         |
|                    | Signed integrity hash per chunk             | `server::integrity::every_ingested_chunk_gets_a_verifiable_tag`                     |
|                    | Signed PII verdict (distinct key)           | `server::classification::pii_verdict_is_signed_with_a_distinct_classifier_key`      |
| Generation         | Exact gate rejects absent/mismatched evidence | `rag::verify` + `rag::pipeline`                                                    |
|                    | Retrieval never crosses `space_id`          | `rag/tests/corpus_pgvector.rs::retrieve_never_crosses_space_or_clearance`           |
|                    | Under-cleared excludes over-level           | same test (clearance filter)                                                        |
|                    | Live-gen gate                               | `server::classification::live_generation_is_gated_by_clearance`                     |
| Answers            | Public projection hides answer              | `core::protocol` projection test                                                    |
|                    | `is_correct` exact set match                | `server::session` tests                                                             |
| Reveal             | Scoring in-memory ≡ Postgres                | `server/tests/store_divergence.rs`                                                  |
|                    | Mastery accumulates                         | `server::session` mastery test                                                      |
|                    | Leaderboard deterministic under concurrency | `server/tests/leaderboard_concurrency.rs` (added this session)                      |
| Breakout/flashcard | Breakout grounded                           | `server/tests/generate_question.rs`                                                 |
|                    | Flashcard deck persisted + retrievable      | `server/tests/flashcard_store_pg.rs` (gated PG) + in-memory                         |

**New modules/seams built:** `oidc`, `auth` (space-tokens), `authz`,
`membership`, `classification`, `integrity`, `flashcard_store`; `corpus` now
scoped by an opaque `RetrievalScope { space_id, max_confidentiality }` (ADR
invariant honored — `rag` never depends on P4).

### §2 gate-wall — complete, CI-enforced (4 jobs)

`check` (build/clippy `-D warnings`/fmt/test) · `supply-chain` (**cargo-deny +
cargo-audit**, `deny.toml` + `.cargo/audit.toml` with justified ignores) ·
`guard` (no machine-local paths / no forbidden internal ref / no secret files) ·
`integration` (real Postgres pgvector + Redis; **coverage ≥ 80%**, last **91.75%**,
via `cargo-llvm-cov`; load test skipped from the instrumented run — perf
assertions are invalid under coverage instrumentation).

### §3 observability — load-test SLOs met

`server/tests/load.rs` (200 real WS participants, `--release`): delivery
p99 ≈ 26 ms (< 200) · reveal ≈ 18 ms (< 500) · answer-submit ≈ 51 ms (< 100),
1000/1000 zero loss. `live_rag` passes against a real model (gemma-31b), but
that positive run does not prove that the source instruction was neutralized.

---

## Blocked — needs external dependencies (not buildable in-session)

### §3 AI-latency SLOs — inference-bound (measured, not assumed)

The generation SLO (`generation p99 < 5 s`, retrieve+generate+verify) is **not
attainable on the available local hardware**, measured via
`server/tests/slo_generation.rs` (the named source, self-skips in CI):

| Model                      | Loadable?                 | Real pipeline (generate + independent verify) |
| -------------------------- | ------------------------- | --------------------------------------------- |
| gemma-4-31b (dense)        | yes                       | ~36 s                                         |
| gemma-4-12b (dense)        | yes                       | **p50 18 s / p99 25 s**                       |
| gemma-4-26b-a4b (fast MoE) | **no — insufficient RAM** | —                                             |

The moat _requires_ the second LLM call (independent grounding verify), so even
the fastest loadable model is ~4–5× over budget. **`< 5 s` needs production-grade
inference** (a faster model, or a machine that loads the a4b MoE). The code path
is correct (`live_rag` proves it); only inference speed is missing. Retrieval-
search (post-HNSW) and ingestion SLOs are achievable but moot until generation is.

### §4 compliance & §5 SP-C — not built

Audit log (0-PII), RGPD erasure cascade, A11y/WCAG, cross-browser e2e (§4) and the
SP-C client (§5) need an audit substrate **and a front end** — see next.

---

## Next: SP-C Increment 1 (the architecture agent's spec)

Spec: `docs/specs/2026-06-28-frontend-dioxus-design-system-design.md`. All-Rust
Rust-first clients consuming Portal client-platform contracts; `rumble-lm-ui` is local to LM-specific components only.

**Inc-1 vertical slice — authenticated personal notebook (web/PWA):**

> Log in via OIDC (Keycloak) → see the solo space → submit a RAG query → render
> the response with cited source cards + a server-authoritative confidentiality
> badge → logout/refresh wired.

- **Crates:** `crates/ui` (`rumble-lm-ui` — LM-specific components consuming Portal tokens/a11y) and future app/join surfaces, both re-using `presto-core` contracts.
- **Auth/transport (honor the invariant):** Auth Code + PKCE → token in
  `HttpOnly; Secure; SameSite=Strict` cookie set server-side at `/auth/callback`;
  the wasm client never reads the cookie; `fetch(..., {credentials:'include'})` +
  server `Sec-Fetch-Site: same-origin` check. Token is a capability, **not** a
  cache (membership stays authoritative — reuses `server::membership`).
- **Server seam to add:** `/auth/callback` (OIDC redirect → cookie; reuses
  `server::oidc::validate_id_token`), a `GET /api/spaces/{id}` and `POST
/api/rag/query` (the latter already exists on P1).
- **Tests:** Dioxus component unit tests (a11y roles, focus trap, dark-mode token
  swap) + Playwright e2e on `[chromium, firefox, webkit]` (login flow, rag query,
  logout, session refresh). Freeze the real OIDC dance with `claude-in-chrome`
  against a local Keycloak dev instance before writing the e2e.
- **Dependency to provide:** a **Keycloak dev instance** (no creds in-repo) — same
  IdP gap that blocks the §1 OIDC _flow_ end-to-end (the _validation_ is already
  tested with mock tokens).

Increments 2 (guest/join + live UI) and 3 (Tauri desktop + offline RAG) follow.

---

## Open items / known caveats

- **AI-gated tests** (`live_rag`, `slo_generation`) self-skip without
  `LOCAL_AI_BASE_URL`/`DATABASE_URL`; run locally with a loopback model + a pgvector container
  (port **5439**, never 5432) to exercise them.
- **`Pf2` HNSW index** (evolution roadmap) not built — retrieval uses a full scan
  (fine at wedge scale; needed for the retrieval SLO at scale, but the column is
  deliberately dimension-free so HNSW requires fixing the embedding dimension).
- **Stable baseline scope** — backend/RAG/live-session gates are green; SP-C front,
  RGPD erasure/audit, and production AI-latency remain tracked open work.
