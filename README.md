# Presto-Matic

A sovereign, self-hostable collaborative learning platform — **NotebookLM × Kahoot**:
auto-generated, _source-grounded_ study content (quiz, flashcards, mind maps,
summaries) delivered in **real-time collaborative sessions** (200+ participants).

- **Sovereign / BYO** — self-host on your own infrastructure with your own AI keys.
  Defaults to Clever Cloud + Clever AI (EU, RGPD).
- **Grounded** — every generated item is traceable to its source, and verified by
  an agentic grounding checker (the wedge: trust).
- **Live** — host a session, participants join by link, answer grounded quizzes,
  watch a live leaderboard and a real-time comprehension heatmap.

> Status: `v0.1` — backend/RAG/live-session stable baseline. The live-session
> tracer bullet is implemented and gated (Biscuit join link, 200 participants,
> grounded generation, real-time aggregation, leaderboard/load SLOs). Product-complete
> front, RGPD erasure/audit, and production AI-latency work remain tracked in `docs/`.

## Workspace

- `crates/core` — shared Rust client/protocol core (→ native via UniFFI, → wasm for web).
- `crates/rag` — ingestion, retrieval, grounded generation, verification, flashcards.
- `crates/server` — backend (axum; HTTP/WebSocket session engine, authz, stores, fanout).

## Stack

Rust · axum / tokio · PostgreSQL + pgvector · Cellar (S3) · Redis / Materia KV ·
Pulsar · Biscuit auth (+ OIDC / Keycloak) · OpenAI-compatible AI (Clever AI default).

## Companion repositories

Adjacent tooling lives in separate repos so Presto-Matic keeps a tight runtime boundary:

- [`memory-card`](https://github.com/constantin-jais/memory-card) — local agentic context, code map, repo memory.
- [`disc-loader`](https://github.com/constantin-jais/disc-loader) — Xberg-backed rich document ingestion worker/service.
- [`vault-inspector`](https://github.com/constantin-jais/vault-inspector) — Scythe-backed SQL audit and Postgres security inspection.
- [`supply-depot`](https://github.com/constantin-jais/supply-depot) — Starmetal-backed sovereign registry proxy / supply-chain POC.
- [`link-cable`](https://github.com/constantin-jais/link-cable) — Rust-first multi-platform distribution substrate with forward-only releases and sovereign install floors.

See [`docs/adr/0003-companion-repositories.md`](docs/adr/0003-companion-repositories.md).

## License

MIT © Constantin Jais
