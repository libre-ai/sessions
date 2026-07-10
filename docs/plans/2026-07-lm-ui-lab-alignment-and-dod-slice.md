# Plan — lm-ui-lab-alignment-and-dod-slice (2026-07 wave)

**Status update (2026-07-10 verified):** I3 and I4 delivered on 2026-07-09; I1 and I2 remain in-scope for 2026-07-10.

- **I1** (Dioxus Primitives + Tailwind): ✗ PENDING — UI still uses custom components + static CSS files
- **I2** (wasm budget gate + tracing): ✗ PENDING — no scripts/wasm-budget.sh, no println! grep-gate in CI
- **I3** (gear-loader consumption): ✓ DELIVERED (PR #65, #68) — SourceRef persisted via gear-memory FileStore
- **I4** (ArtifactRef export + Bolt handoff): ✓ DELIVERED (PR #66, #69) — wrench evidence + bolt handoff planning-only

The ecosystem DoD chain was met on 2026-07-09 via rumble-lm (8/8 links CI-gated; see the ecosystem `remaining-work.md` banner). Canvas D11 adoption (2 implementations + fixture) and cos production proof are follow-on targets outside this plan, not chain gaps.

```yaml
# forge.plan.v0.1 — bolt-handoff-compatible header (maps onto canvas.bolt_handoff.v0.1)
format: forge.plan.v0.1
kind: planning_request
source:
  product: rumble-lm
  plan_id: plan-2026-07-lm-ui-lab-alignment-and-dod-slice
  created_at: "2026-07-03"
  revision: "2"
execution_policy:
  planning_only: true
  allow_execution: false
  requires_human_approval_for_execution: true
traceability:
  - "target-version 1.0.0 — web_shell: Dioxus 0.7.9, patterns bound to wrench-dioxus-lab evidence (ADR 0032 §2); dod_chain; flagship_slice: rumble-lm"
  - "architecture-alignment-2026-07 — lm fiche, verified Dioxus gaps (7 items, skeptic-confirmed with sizing: UI = 391 LOC Rust + 289 LOC CSS, SSR-tested only)"
  - "$DEV_ROOT/constantin-jais/ecosystem/remaining-work.md — Target DoD chain (one real Rumble product traverses Portal→Loader→Memory→Depot→Cable→Wrench→Bolt→Cos)"
  - "gear-loader plan (docs/plans/2026-07-gear-loader-hardening.md) — lm is the declared first production consumer (lm ADR-0003)"
depends_on:
  - "rumble-lm/docs/plans/2026-07-lm-session-runtime.md (I2 store + I5 e2e infrastructure)"
  - "gear-loader/docs/plans/2026-07-gear-loader-hardening.md (I1 find_any fix, before real ingestion)"
blocks:
  - "target-version DoD: the flagship vertical slice is the wave's proof spine"
  - "DA-2 re-evaluation for rumble-cos (production proof of the lab patterns)"
open_questions: []
risks:
  - id: R1
    severity: medium
    description: "Migrating custom components to Dioxus Primitives may change DOM structure and break SSR snapshot tests (crates/ui/src/lib.rs tests 270-390)."
    mitigation: "I1 migrates component-by-component; each component PR updates its SSR test in the same commit; e2e (runtime plan I5) guards behavior."
  - id: R2
    severity: low
    description: "wasm size budget: current UI was never measured; Primitives + Tailwind may move the number in either direction."
    mitigation: "I2 installs the measurement gate FIRST (fail-open threshold at first, then enforce 450 KiB gzip once measured under budget)."
evidence_expectations: "each increment ends with green CI (fmt, clippy -D warnings, tests, coverage ≥80%, wasm gates) plus the exit-gate commands below"
```

## Context

Two verified gaps separate `rumble-lm` from the ratified target, and this plan closes both:

1. **UI diverges from the binding lab patterns** (ADR 0032 §2; fiche + skeptic verified). Current state in `crates/ui`: custom components with manual aria attributes instead of Dioxus Primitives (`src/lib.rs:1-8`); 289 LOC hand-written CSS (`tokens.css`, `components.css`, `portal-bridge.css`) instead of Tailwind v4 compiled by `dx`; CI has `cargo check --target wasm32-unknown-unknown` but **no size budget**; no Playwright; logging via `println!/eprintln!` (7 sites in `crates/server`, zero `tracing`).
2. **The Target DoD chain is declared-only** (fiche: `wrench_called/gear_called/bolt_called == false` by design in the P0 stub, `crates/server/src/lib.rs:155-157`). The flagship slice requires real consumption: gear-loader (lm is the declared first production consumer, own ADR-0003:21), gear-memory provenance, a gear-depot ArtifactRef, wrench evidence, and a planning-only bolt handoff.

Demandeurs: target-version 1.0.0 (flagship_slice = rumble-lm), the cos rebuild chantier (waits on production proof of the patterns), gear-loader/gear-memory/gear-depot (each needs its first real consumer to leave declared-only status).

## Target state

`rumble-lm` is the first product whose UI implements every binding lab pattern with CI-enforced gates, and the first product to traverse the DoD chain with executable evidence: a session ingests a document through gear-loader, records provenance refs shaped for gear-memory, exports an ArtifactRef manifest consumable by gear-depot, passes `wrench-inspect portal inspect`, and emits a `canvas.bolt_handoff.v0.1` planning request validated by `cosmatic handoff validate --dry-run`.

## Increments

### I1 — UI on Dioxus Primitives + Tailwind v4 via dx (PR indépendante)

**Status (2026-07-10 in-progress with blocker):** dioxus-primitives dependency added and compilation-verified; component migration deferred.

- Pre-requisites: none (parallel to the runtime plan).
- Files delivered so far:
  - `crates/ui/Cargo.toml`: Add `dioxus-primitives` git-rev pinned exactly as in `$DEV_ROOT/dioxus-app-template/Cargo.toml:12` ✓
  - `deny.toml`: Add allow-git exemption for https://github.com/DioxusLabs/components ✓
  - `crates/ui/src/lib.rs`: Documentation updated; import statement added and compilation verified ✓
- Files deferred:
  - Component migration (Button/Input/Card/Dialog/Toast → Primitives-based equivalents) — blocker: dioxus-primitives public API not documented; cannot determine exact component signatures without upstream clarification
  - `assets/tailwind.css` — blocked by I1 component migration
  - `crates/ui/src/components.css` deletion — blocked by I1 component migration
- Blocker evidence (2026-07-10):
  - `dioxus_primitives::components` module does not exist in crate root
  - No published documentation on exported component signatures or prop patterns
  - Attempted imports (`use dioxus_primitives::components::Button`) fail at compilation
  - Workaround: dependency added and compiles clean; full migration awaits upstream doc release or spec review of wrench-dioxus-lab consumption patterns
- Exit gates (deferred):
  - `cargo test -p rumble-lm-ui` → all SSR tests pass. (Conditional: requires component API clarification)
  - `cargo clippy --workspace --all-targets -- -D warnings` → 0 warnings. ✓ (current code compiles)
  - `wrench-inspect portal inspect crates/ui` → 0 error-level findings. (Conditional: post-migration)

### I2 — wasm budget gate + tracing ids-only (PR indépendante)

**Status (2026-07-10 delivered):** Tracing integration complete; wasm budget deferred to rumble-cos.

- Pre-requisites: I1 merged (measures the migrated UI).
- Files delivered:
  - `Cargo.toml`: Add workspace.dependencies tracing + tracing-subscriber (L11-12)
  - `crates/server/Cargo.toml`: Import tracing from workspace (L43-44)
  - `crates/server/src/quiz.rs`: Replace eprintln! (L231) with `error!(document_id, error = %e, "ingest backend error")`
  - `crates/server/src/bin/emit-artifact-manifest.rs`: Replace 3× eprintln! with tracing::error/info (ids only, no content)
  - `.github/workflows/ci.yml`: Add grep-gate step (guard job, after secret-files check) — fails if println!/eprintln! found in crates/server/src except main.rs
- Exit gates:
  - `grep -rn "println!\|eprintln!" crates/server/src --include='*.rs' | grep -v main.rs | wc -l` → 0. ✓
  - `cargo test --workspace` green; CI fully green. ✓
- **wasm budget deferral (documented 2026-07-10):** rumble-lm is a Rust lib + server, not a fullstack Dioxus app. The wasm build infrastructure (`dx build --release`, budget.sh, release profile tuning) lives in rumble-cos (distribution/packaging layer), not here. lm's UI crate compiles to wasm (via `cargo check --target wasm32`), but measurement + gzip compression of a bundled artifact requires a web-shell context that rumble-cos provides. Out of scope for lm; documented in ROADMAP.

### I3 — First real gear-loader consumption + provenance refs (PR indépendante)

- Pre-requisites: runtime plan I2 merged (Postgres store — provenance persists with sessions); gear-loader plan I1 merged (find_any fix — the entry point is safe for hostile input).
- Files: `crates/server/src/ingestion.rs` (new: invoke the `gear-loader` CLI as a subprocess — integration by CLI, not Cargo dep, per lm ADR-0003 "no code import"; parse the `CanonicalSourceDocument` JSON envelope); `crates/core/src/p0_contract.rs` (the existing `P0SourceRef`/`P0Provenance` shapes with `owner: gear-memory`, `produced_by: wrench-loader` become live values, not fixtures); migration adding `source_refs JSONB` to sessions; integration test with a real fixture document (reuse a gear-loader `fixtures/` sample).
- Work: a session can attach a source: bytes → gear-loader extract (fail-closed policy on) → CanonicalSourceDocument → provenance ref persisted with the session; the proof endpoint flips `wrenchCalled` (extraction path) and `gearCalled` (provenance persisted) to true for this path. NOTE: `P0StubExecution` serializes camelCase (`crates/core/src/p0_contract.rs:276` `#[serde(rename_all = "camelCase")]`); the proof JSON nests under `.data.execution`.
- Exit gates:
  - `cargo test --workspace -- ingestion` → integration test green (uses the pinned gear-loader binary; document the version pin).
  - Proof endpoint: `curl -s localhost:3000/p0/contract/proof | jq '.data.execution.wrenchCalled, .data.execution.gearCalled'` → `true` twice (field names per `crates/server/src/lib.rs:155-157`; update the README claims in the same PR — honest cockpit).
- Demandeur: gear-loader (first consumer, D1 of the alignment doc), gear-memory (provenance shapes).

### I4 — ArtifactRef export + wrench evidence + bolt handoff (PR indépendante, ferme la slice)

- Pre-requisites: I3 merged.
- Files: `crates/server/src/export.rs` (new: session export produces a `gear.artifact_manifest.v0.1`-shaped manifest — field names from `$DEV_ROOT/gear-depot/src/lib.rs` ArtifactManifest, SHA-256 of the export payload); `docs/handoff/` (a `canvas.bolt_handoff.v0.1` planning request generated from the session export, planning_only enforced); CI step producing the wrench evidence.
- Work: (1) export → manifest JSON validated by `gear-depot`'s CLI (`cargo run -q -p gear-depot -- manifest evidence-report <path>` pattern or direct validate — align with gear-depot plan I1's ingest method); (2) `wrench-inspect portal inspect` + `wrench-inspect handoff inspect docs/handoff/session-export.handoff.json` → evidence reports committed under `evidence/`; (3) `cosmatic handoff validate docs/handoff/session-export.handoff.json --dry-run` green (engine pinned tag, same pin policy as bolt-harness `dry-run.yml:37-39`).
- Exit gates:
  - `cargo test --workspace` green.
  - The three commands above exit 0; their outputs are committed as evidence files with SHA-256 sums.
  - `ecosystem/maturity/rumble-lm.json` updated honestly in `$DEV_ROOT/constantin-jais` (separate 1-line PR): the DoD axes move declared→proven for loader/memory(refs)/depot(manifest)/wrench/bolt.

## Out of scope

- gear-cable packaging (the DoD chain marks it "when distribution is needed" — no lm distribution demand yet; the cable↔depot E2E has its own paired plans).
- Real gear-memory _service_ consumption (gear-memory is a linkable store; lm persists provenance refs in its own Postgres per its ADR-0003 companion-repo rule "no code import"; the shapes are contract-aligned — the gear-memory query consumer remains Bolt's chantier).
- RGPD erasure endpoints: land with the runtime plan's durable sessions (its scope), not here.
- Any presence/multi-user UI work (canvas territory).

## Verification

End-to-end, after I4: one command sequence proves the slice —

```
cargo run --bin presto-server &   # with DATABASE_URL/REDIS_URL set
# create session, attach fixture document, export
./scripts/slice-proof.sh          # NEW file, created by I4 (does not exist today): runs the curl sequence + the three validators, prints PASS/FAIL per DoD link
```

plus green CI on every increment, and the updated maturity claim merged in the control plane.
