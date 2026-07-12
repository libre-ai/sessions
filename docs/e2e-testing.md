# End-to-End Testing Guide

## Overview

The `e2e/` directory contains Playwright tests for the current minimal web client and session runtime:

- landing page renders the host entry point;
- `POST /sessions` returns workspace-identity facts (`tenant_local`, `workspace_{session_id}`);
- a browser host creates a session;
- a browser participant joins through the generated link;
- the host opens a fixture question;
- the participant answers;
- the host reveals the leaderboard.

The targeted `owner-shell.spec.ts` smoke additionally opens the Dioxus shell at `/app` with a 390×844 mobile viewport, navigates its owner routes, verifies accessible navigation/sticky query placement, and confirms that the shell creates no browser storage or service worker. `owner-notebook.spec.ts` mocks the owner APIs to deterministically prove rendering, rejection, disabled submit and current-space retry states; real cookie authz, cross-space, clearance and CSRF remain blocking Rust router tests. When `KEYCLOAK_E2E=1`, `owner-auth-keycloak.spec.ts` is the no-mock real mobile gate: login → `/api/me` → personal space → `/app/notebook` → real RAG handler answer/citation → refresh → logout against the pinned development Keycloak.

These tests exercise the deployed browser surface. Deeper protocol and scoring cases remain in Rust integration tests.

## Prerequisites

- Node.js 18+;
- Rust toolchain;
- browser dependencies installed by Playwright;
- un bundle owner généré et vérifié avec `./scripts/build-owner-app.sh` (Dioxus CLI 0.7.9).

Postgres and Redis are optional for the current e2e flow: when `DATABASE_URL` / `REDIS_URL` are absent, the server uses in-memory state and fixture content. CI still provides Postgres + Redis because other integration jobs use them.

For the real owner auth gate, Docker Compose, `curl`, and Python 3 are also required. No Keycloak credential is tracked: `scripts/keycloak-dev.sh` creates random values in the ignored `dev/keycloak/.env` with mode 0600.

## Setup

```bash
cd e2e
npm ci
npx playwright install
```

## Run tests

```bash
./scripts/build-owner-app.sh
cd e2e
npm test

# Owner shell and mocked deterministic UI states
npx playwright test tests/owner-shell.spec.ts tests/owner-notebook.spec.ts --project=chromium

# Real Keycloak + real notebook API path (after the setup below)
npx playwright test tests/owner-auth-keycloak.spec.ts --project=chromium
```

## Real Keycloak mobile gate

Start the reproducible Keycloak 26.5.2 realm and export its generated test credentials without printing them:

```bash
./scripts/keycloak-dev.sh up
set -a
source dev/keycloak/.env
set +a

export OIDC_ISSUER=http://localhost:8081/realms/rumble-lm-dev
export OIDC_CLIENT_ID=rumble-lm-owner
export OIDC_REDIRECT_URI=http://localhost:3000/auth/callback
export OWNER_AUTH_SINGLE_INSTANCE=1
export KEYCLOAK_E2E=1

./scripts/build-owner-app.sh
cd e2e
npm ci
npx playwright install chromium
npx playwright test tests/owner-auth-keycloak.spec.ts --project=chromium
```

The test first proves that an OIDC callback initiated by browser A is rejected when presented by isolated browser B. It then uses a 390×844 touch profile, asserts the exact cookie properties through the browser context, proves the cookie is absent from `document.cookie` and web storage, reloads the page, then submits the real same-origin logout form. Tracing is explicitly disabled for this file so credentials and the short-lived protocol callback cannot enter a Playwright trace. Stop or fully rotate the local environment with:

```bash
./scripts/keycloak-dev.sh down
./scripts/keycloak-dev.sh reset  # also deletes generated credentials
```


Playwright will:

1. vérifier le bundle owner présent, puis démarrer le serveur Rust depuis la racine avec `PORT=3000`;
2. wait for `http://localhost:3000`;
3. run `tests/*.spec.ts`;
4. generate an HTML report in `e2e/playwright-report/`.

To run against an already-started server:

```bash
PORT=3000 cargo run --bin presto-server
cd e2e
BASE_URL=http://localhost:3000 npm test
```

## Debugging

```bash
npm run test:debug
npm run test:headed
npm run test:ui
npx playwright show-report
```

## CI Integration

The `.github/workflows/ci.yml` includes an `e2e` job that:

1. télécharge le paquet owner construit une seule fois depuis le checkout courant et vérifie sa liste de fichiers et tous ses SHA-256;
2. starts Postgres 16+pgvector + Redis 7;
3. builds `presto-server`, qui embarque donc exactement ce paquet vérifié;
4. installs Playwright dependencies with `npm ci`;
5. runs `cd e2e && npm test`;
6. uploads the HTML report as a CI artifact.

Tests must pass before merge.

The real Keycloak browser test is a **documented manual pre-merge gate**, not CI-blocking: the default GitHub job has no nested container orchestration and no secret credential input by design. When `KEYCLOAK_E2E=1`, Playwright refuses to reuse a server already listening on port 3000 and always starts the binary from the current checkout. The blocking deterministic protocol integration is `owner_auth::tests::full_login_projects_dtos_bootstraps_once_replays_safely_and_logs_out`, `owner_auth::tests::callback_is_bound_to_initiating_browser_and_consumed_on_mismatch`, and the other adversarial owner-auth tests. They start an in-process HTTP OIDC provider and exercise discovery, token POST/PKCE, RS256/JWKS validation and the complete Axum router. CI runs them in `cargo test --workspace --all-features`. A maintainer records the separate manual Keycloak result on the pull request before merge; it must not be described as CI-blocking.
