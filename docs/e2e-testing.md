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

The targeted `owner-shell.spec.ts` smoke additionally opens the Dioxus shell at `/app` with a 390×844 mobile viewport, navigates its owner routes, verifies accessible navigation/sticky query placement, and confirms that the shell creates no browser storage or service worker.

These tests exercise the deployed browser surface. Deeper protocol and scoring cases remain in Rust integration tests.

## Prerequisites

- Node.js 18+;
- Rust toolchain;
- browser dependencies installed by Playwright;
- un bundle owner généré et vérifié avec `./scripts/build-owner-app.sh` (Dioxus CLI 0.7.9).

Postgres and Redis are optional for the current e2e flow: when `DATABASE_URL` / `REDIS_URL` are absent, the server uses in-memory state and fixture content. CI still provides Postgres + Redis because other integration jobs use them.

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

# Owner shell smoke only
npx playwright test tests/owner-shell.spec.ts --project=chromium
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
