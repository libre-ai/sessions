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

## License

MIT © Constantin Jais
