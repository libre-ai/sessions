# Sessions Canonical Agent Rules

## Purpose

Sessions is the couche-1 product for facilitators preparing and running
learning/facilitation sessions grounded in cited sources, with explicit,
bounded delegation and end-to-end tooled GDPR rights.
Doctrine lives upstream: https://raw.githubusercontent.com/libre-ai/governance/main/docs/README.md

## Domain doctrine

- No unsourced content: session material must trace to cited sources.
- No unlimited retention: retention windows are declared and enforced
  through the `rgpd-kit` machinery (access, erasure, restriction).
- Delegation is explicit and bounded — never implicit.
- Bricks and contracts this repo depends on (`data`, `rgpd-kit`,
  `web-platform`, `testing`, `contracts`, `governance`) are consumed pinned
  by SHA, never redefined here.

## Commands

- `bun install` — install workspace dependencies.
- `bun run test` — runs the `apps/sessions` test suite.
- `bun run lint` — Biome, CI mode.
- `bun run check` — the full gate chain (toolchain, app tests, secret scan,
  personal-data boundary, lint); run before pushing.

## Working here

- Security > quality > performance > completeness, in that order on conflict.
- Check real state before editing: `git status --short` and `bun run check`.
- English for code, comments and this file; French stays the human
  conversation language elsewhere.
- Never commit a machine-local absolute path; use repo-relative paths or `~`.
