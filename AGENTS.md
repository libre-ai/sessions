# AGENTS.md

Canonical agent-context surface for this repository. `CLAUDE.md` is a minimal adapter that imports this file.

## Purpose

Sessions is source-grounded collective learning and facilitation: a group works around sourced materials with explicit roles (facilitator, participant, observer), audience rules for each contribution, and a human approval gate before any shared outcome is published. Never a silent synthesis; never an export that reveals private input by default.

## Scope / Non-scope

- **Reserved home.** This repository is the public reserved home of Sessions. The product is being rebuilt in the canonical base repository [`libre-ai/libre-ai`](https://github.com/libre-ai/libre-ai) (multi-repo topology, [ADR-0008](https://github.com/libre-ai/libre-ai/blob/main/docs/adr/0008-multi-repo-target-topology-and-brand.md)); it reopens as the real product repository when the owner activates it (wave 4).
- The legacy implementation carried here (Rust workspace `crates/{app,core,join,rag,server,ui}`, Clever Cloud build/smoke scripts, Playwright e2e suite) is **frozen for reference**.
- Non-scope: new product development in this repository until activation.

## Commands

Verified against `Cargo.toml`, `scripts/`, and `e2e/package.json`:

- Rust workspace: `cargo test --workspace` (members: `crates/app`, `crates/core`, `crates/join`, `crates/rag`, `crates/server`, `crates/ui`).
- e2e (from `e2e/`): `npm run test` (Playwright; variants `test:debug`, `test:headed`, `test:ui`, `playwright:install`).
- App packaging and deployment scripts in `scripts/`: `build-owner-app.sh`, `build-join-app.sh`, `package-owner-app.sh`, `package-join-app.sh`, `verify-owner-app.sh`, `verify-join-app.sh`, `clever-pre-build.sh`, `clever-smoke.sh`, `clever-staging-preflight.sh` (each with a matching `test-*.sh`), `keycloak-dev.sh` (local Keycloak, see also `dev/keycloak`).

## CI gates

Three workflows. The check names below are the exact strings branch protection
matches on `main`; renaming a job means updating that protection in the same
move. **Running is not blocking**: a job blocks a merge only once its name is in
the repository's `required_status_checks`.

| Check                          | Required                | Workflow                                | Enforces                                                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------ | ----------------------- | --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Tracked tree hygiene`         | yes                     | `.github/workflows/context-hygiene.yml` | Tree-level hard-fail invariants over tracked files: no private identifier, no machine-local path (marker `allow-local-path` exempts a line), no offending symlink target, no undeclared duplicate file contents (`scripts/check-duplicate-contents.sh`), no advisory waiver that is undated, expired, or names a crate absent from `Cargo.lock` (`scripts/check-advisory-waivers.sh`). |
| `REUSE compliance`             | yes                     | `.github/workflows/licensing.yml`       | Every tracked file carries an unambiguous copyright and licence declaration, per `REUSE.toml`.                                                                                                                                                                                                                                                                                         |
| `Rust quality gates`           | **blocked — see below** | `.github/workflows/rust.yml`            | `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -D warnings`, `cargo test --workspace --all-targets --all-features`, and `wasm32-unknown-unknown` portability of the client crates. Builds the owner and join Dioxus bundles first, because `crates/server/build.rs` refuses to compile without them and they are gitignored.                              |
| `Supply-chain policy`          | **blocked — see below** | `.github/workflows/rust.yml`            | `cargo deny check bans licenses sources` — the half of the policy that reads only `Cargo.lock`, `deny.toml` and immutable per-version crate metadata, so its verdict is a function of the commit.                                                                                                                                                                                      |
| `Advisory watch (report only)` | no — never add          | `.github/workflows/rust.yml`            | `cargo deny check advisories` and `cargo audit`. Consults the RustSec database and crates.io yank state, both of which move without a commit here; it reports and never blocks, so that a required check can never go red on someone else's publication.                                                                                                                               |

Both of the two checks that are required today reason over the _tracked tree_;
neither compiles a line of Rust. `Rust quality gates` and `Supply-chain policy`
are the correctness half, restored in `rust.yml` after `ci.yml` and
`security.yml` were retired with the legacy product CI and nothing replaced
them.

**Why the two new checks cannot be required yet.** `Cargo.toml` takes
`gear-loader` and `gear-memory` from `https://github.com/libre-ai/context-kit`
at rev `f0c10bf3`, and that repository no longer exists — 404 anonymously and
authenticated, absent from the organisation listing, absent from crates.io.
Nothing that resolves the dependency graph can run on a clean machine, so both
checks are red in CI while passing locally off a warm `~/.cargo/git` cache.
Restore, relocate or vendor the source first; then add these exact strings to
`required_status_checks`:

```
Rust quality gates
Supply-chain policy
```

Never add `Advisory watch (report only)`: it consults the RustSec database and
crates.io yank state, which move without a commit here, and a required check
that can redden `main` on someone else's publication is one people learn to
ignore.

## Links

- [README](README.md) · [Français](README.fr.md)
- [docs/README.md](docs/README.md) — documentation index (adr, contracts, db, deploy, evidence, specs, security, status)
- [docs/product-readiness.md](docs/product-readiness.md) — canonical readiness cockpit
- [docs/e2e-testing.md](docs/e2e-testing.md), [docs/pwa-testing.md](docs/pwa-testing.md)
- [ROADMAP.md](ROADMAP.md), [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md)
