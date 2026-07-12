# Goal — Full-spec session coverage

- **Status:** Active — minimal session goal delivered; full-spec goal remains open
- **Date:** 2026-06-28
- **Objective:** A Presto-Matic live session runs end to end with **every spec element active and proven** — the grounding moat, typed questions, mastery/SRS, breakouts, the SP-A authz substrate, SP-B classification/clearance, and the SP-C client contracts — under the cross-cutting gate-wall, SLOs, and sovereignty constraints.
- **Scope:** the unified design corpus — see [`../README.md`](../README.md) (built differentiators + SP-A/B/C + evolution P0/P1 hardening). Out of scope: items explicitly marked "defer" in the evolution spec until their increment lands.
- **How "done" is judged:** every **gate** KPI is binary green (blocking, with attached evidence — "nothing merges red"); every **observability** KPI meets its SLO under the load test; every **completeness** element has `{code + tests + API docs + example/ADR}`. A KPI is only credited when its **source** (a named test / metric / query) is green.

---

## Delivery status (2026-06-29, stable backend/RAG/live-session baseline)

| Section                  | State                        | Notes                                                                                                                    |
| ------------------------ | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| **§1 moat**              | ✅ **19/19 proven**          | every KPI has a named green test; AI-gated ones (`live_rag`) proven locally                                              |
| **§2 gate-wall**         | ✅ **complete, CI-enforced** | check · cargo-deny + cargo-audit · guard-scan · integration (coverage 91.75%)                                            |
| **§3 SLOs (load test)**  | ✅ **met**                   | delivery 26ms · reveal 18ms · answer-submit 51ms (`load.rs`, 200 participants)                                           |
| **§3 SLOs (AI latency)** | ⛔ **inference-bound**       | generation pipeline measured ~20s on the fastest loadable model; `< 5s` needs production inference (`slo_generation.rs`) |
| **§4 compliance**        | ◻️ not started               | audit 0-PII, RGPD erasure, A11y, cross-browser e2e                                                                       |
| **§5 / SP-C front**      | ◻️ next                      | Inc-1 plan in [`../status/2026-06-29-session-handoff.md`](../status/2026-06-29-session-handoff.md)                       |

The **minimal session goal** below (moat §1 + §2 + the §3 SLOs) is met but for the
AI-latency subset of §3, which is hardware-bound. Full "Done" additionally needs
production inference + SP-C + the §4 compliance surface.

---

## 1. Functional KPIs by session stage (moat first)

| Stage (spec)                     | KPI                                                                       | Target          | Source                 |
| -------------------------------- | ------------------------------------------------------------------------- | --------------- | ---------------------- |
| **Join / auth** (SP-A)           | OIDC reject on invalid `iss/aud/exp/nonce/sig`                            | 100%            | OIDC test              |
|                                  | Space-A token opens space B                                               | 0%              | `requested_space` test |
|                                  | Anonymous live join-link still works after `session→space` generalization | OK              | e2e test               |
|                                  | 404 body: _not-found_ == _forbidden_ (anti-enumeration)                   | identical       | SP-A test              |
|                                  | Revoked member denied on sensitive op despite unexpired token             | < cache window  | integration test       |
| **Ingestion** (P1/P11, SP-B, S1) | Source-absent answer rejected despite provider `supported=true`           | 100%            | lexical regression     |
|                                  | Signed `integrity` hash present per chunk                                 | 100%            | SP-B inc-1 test        |
|                                  | Signed PII verdict (distinct classifier key)                              | 100%            | SP-B inc-3 test        |
| **Grounded generation** (moat)   | Exact gate rejects absent/mismatched evidence                             | 100% on the set | verify test            |
|                                  | Retrieval never crosses `space_id`                                        | 0 leak          | isolation test         |
|                                  | Under-cleared member excludes over-level chunks                           | 100%            | SP-B anti-exfil test   |
|                                  | Live-gen gate: generation never exceeds the audience ceiling              | 0 leak          | SP-B inc-3 test        |
| **Answers** (typed questions)    | Public projection hides answer + `source_section_ids`                     | 100%            | protocol test          |
|                                  | `is_correct` = exact set match (multi-select)                             | 100%            | session test           |
| **Reveal** (scoring/mastery)     | Scoring in-memory == Postgres                                             | identical       | divergence test (O3)   |
|                                  | Mastery accumulates across rounds                                         | OK              | `multi_round` test     |
|                                  | Leaderboard deterministic under concurrency                               | OK              | `concurrent` test      |
| **Breakouts / flashcards**       | Breakout grounded in its section                                          | 100%            | pipeline test          |
|                                  | Flashcard deck persisted + retrievable                                    | OK              | Prod2 test             |

## 2. Hard gates (binary green/red — the gate-wall, nothing merges without)

- `cargo build --workspace --all-targets` = **0 errors** · `clippy -D warnings` = **0 warnings** · `fmt --check` = clean
- `cargo test --workspace` = **100% pass** ; line coverage **≥ 80%** (CI-blocking)
- **`cargo-deny`** (licenses MIT/Apache/MPL only) + **`cargo-audit`** (0 vulns) = pass [evolution Sec2]
- **0** machine-local paths · **0** internal/proprietary-employer reference · **0** secret in the diff (guard scan)
- Real-infra gated tests green: Postgres multi-instance, Redis multi-instance, **`live_rag`** (real provider)

## 3. Observability KPIs (measured, with SLO)

- Fanout delivery **p99 < 200 ms**, zero loss (proven, `load.rs`)
- Question generation **p99 < 5 s** (retrieve+generate+verify, real model) · Retrieval **p99 < 50 ms** (post-HNSW index)
- Answer-submit **p99 < 100 ms** · Reveal **p99 < 500 ms**
- Ingestion: N-page doc **< 10 s** ; chunks/s throughput
- Error rate **< 0.1%**/op · Revocation propagation **< 10 s** (cache window)
- **AI cost**: tokens/session tracked · Concurrent participants **≥ 200** (proven), higher target
- **Moat health**: grounding-verifier rejection rate (observed metric, not a threshold)

## 4. Sovereignty / compliance KPIs (DoD)

- **100%** deps OSS (MIT/Apache/MPL) · **EU** residency (Clever AI sous contrat, ou runtime local)
- Audit: **100%** of sensitive actions logged · **0 PII** in logs **and** audit (`actor_sub` pseudonymous, Art. 17(3) retention basis)
- RGPD: erasure cascades to chunks/embeddings + audit ; deletion endpoint present
- A11y (SP-C): **WCAG** conformance (level TBD) · cross-browser e2e **chromium/firefox/webkit** green

## 5. Completeness matrix (DoD per element)

For each spec element — **% with `{code + tests + API docs + example/ADR}`**:

- the 4 built differentiators · SP-A inc-1/2/3 · SP-B inc-1/2/3 · SP-C inc-1/2/3 · evolution P0/P1.

---

## Minimal session goal (the high-signal subset)

If the operation needs a smaller first target, the minimal target includes the **moat** rows of §1 (lexical fail-closed rejection, space isolation, live-gen gate, grounding) **+ all of §2 (gate-wall) + the §3 SLOs**. This does not claim prompt-injection security completeness: a security-sensitive `Grounded` result still requires independent server-side approved claims.
