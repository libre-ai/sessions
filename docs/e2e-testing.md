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

These tests exercise the deployed browser surface. Deeper protocol and scoring cases remain in Rust integration tests.

## Prerequisites

- Node.js 18+;
- Rust toolchain;
- browser dependencies installed by Playwright.

Postgres and Redis are optional for the current e2e flow: when `DATABASE_URL` / `REDIS_URL` are absent, the server uses in-memory state and fixture content. CI still provides Postgres + Redis because other integration jobs use them.

## Setup

```bash
cd e2e
npm ci
npx playwright install
```

## Run tests

```bash
cd e2e
npm test
```

Playwright will:

1. start the Rust server from the workspace root with `PORT=3000`;
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

1. starts Postgres 16+pgvector + Redis 7;
2. builds `presto-server`;
3. installs Playwright dependencies with `npm ci`;
4. runs `cd e2e && npm test`;
5. uploads the HTML report as a CI artifact.

Tests must pass before merge.
