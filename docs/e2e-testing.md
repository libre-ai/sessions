# End-to-End Testing Guide

## Overview

The `e2e/` directory contains Playwright tests for session lifecycle validation:

- Host creates session, generates join link
- Participants join via token
- Participants submit answers
- Host reveals scores; leaderboard appears
- Error cases (invalid token, etc.)

## Prerequisites

- Node.js 18+ (for npm)
- Live Postgres 16+ + Redis 7 (running locally or in Docker)
- Presto-server compiled (`cargo build --bin presto-server`)

## Setup

### 1. Install dependencies

```bash
cd e2e
npm install
npx playwright install
```

### 2. Set up environment

```bash
# Copy template
cp e2e/.env.example e2e/.env

# Update if needed (e.g., if server is not on localhost:3000)
# By default, Playwright config starts the server automatically.
```

### 3. Start server (manual mode, optional)

If you want to run tests against a pre-started server:

```bash
# Terminal 1: Start Postgres + Redis (docker-compose or local)
# Terminal 2: Start server
cargo run --bin presto-server

# Terminal 3: Run tests
cd e2e
npm test
```

### 4. Run tests (auto-start mode, recommended for CI)

Playwright config is set to auto-start the server. Just run:

```bash
cd e2e
npm test
```

Playwright will:

1. Start `cargo run --bin presto-server` if not running
2. Wait for server to be ready on http://localhost:3000
3. Run all tests in `tests/*.spec.ts`
4. Generate HTML report in `playwright-report/`

## Debugging

### Run tests with Playwright Inspector

```bash
npm run test:debug
```

### Run tests in headed mode (see browser)

```bash
npm run test:headed
```

### Run tests with UI Mode (interactive)

```bash
npm run test:ui
```

### View last test report

```bash
npx playwright show-report
```

## CI Integration

The `.github/workflows/ci.yml` includes an `e2e` job (see Increment I5 exit gates) that:

1. Starts Postgres 16+pgvector + Redis 7
2. Builds presto-server
3. Runs `cd e2e && npm install && npm test`
4. Uploads HTML report as CI artifact

Tests must pass (exit code 0) before PR merge.

## Writing new tests

See `tests/session.spec.ts` for examples. Key patterns:

- Use `test.describe()` for grouping
- Use `test.beforeEach()` for setup
- Use Playwright locators (`page.locator()`, `page.click()`) for UI interaction
- Use `expect()` for assertions
- Use `await page.waitForTimeout()` for delays (better: use event-driven waits)

Docs: https://playwright.dev/docs/intro
