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

- `Context hygiene` (`.github/workflows/context-hygiene.yml`) — the only workflow in this repository.

## Links

- [README](README.md) · [Français](README.fr.md)
- [docs/README.md](docs/README.md) — documentation index (adr, contracts, db, deploy, evidence, specs, security, status)
- [docs/product-readiness.md](docs/product-readiness.md) — canonical readiness cockpit
- [docs/e2e-testing.md](docs/e2e-testing.md), [docs/pwa-testing.md](docs/pwa-testing.md)
- [ROADMAP.md](ROADMAP.md), [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md)
