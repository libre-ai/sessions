# Presto-Matic — Design Spec (v0)

## Positioning

**NotebookLM × Kahoot, sovereign & self-hostable.** Auto-generated, _source-grounded_
study content delivered in **real-time collaborative sessions** (200+ participants).

**Wedge** = grounded auto-generated content × real-time live collaboration ×
sovereign/BYO — the intersection NotebookLM (nascent live collab) and Kahoot/Wooclap
(hand-authored, ungrounded content) each miss. **Moat = grounded-trust +
sovereignty + live-collab**, not any single feature. We do **not** chase NotebookLM
content-parity (unwinnable against Google's Veo-class output).

## Users & core loop

A **host** (teacher/trainer) prepares a corpus (uploads documents), then runs a
**live session**: participants **join by link**, answer **grounded** quizzes, and the
host sees a **live leaderboard** and a **real-time comprehension heatmap**. After the
session, each participant gets a **personalized follow-up** (spaced-repetition).

## Scope

### v1 content (wedge-first)

Grounded **quiz / flashcards / mind map / summary** + **source-cited RAG chat**.

### Deferred (off-wedge — solo consumption)

Podcast/audio overview, video overview (**Cinematic cut**), infographic, data table.
Revisit only after the live-collab wedge is proven.

### Differentiators (roadmap)

1. **Grounding-verifier agent** — anti-hallucination gate; every generated item is
   verified as supported by the corpus. This is the **bridge to the agentic harness**
   (cos-matic) and the source of educational trust. Most original feature.
2. **20+ auto-generated grounded question types** (Wooclap-style, generated from sources).
3. **Mastery model + personalized SRS follow-up** — in-session knowledge tracing →
   targeted spaced-repetition flashcards per participant.
4. **Real-time confusion heatmap + AI breakout groups** — host sees which source
   sections confuse the room; breakouts get a grounded AI facilitator.

## Architecture

### Clients

A single **Rust core** (`crates/core`: session protocol, shared state, reconnection,
Biscuit handling) compiled to **native via UniFFI** (host app / installed users) **and
wasm** (web participant client, join-by-link). Web (wasm) first for the tracer-bullet.

### Backend

`crates/server` — **Rust (axum + tokio)**. WebSocket session engine, RAG, generation
orchestration. One self-hostable artifact; deployable to Clever Cloud (reads `PORT`).

### Auth

- **Authn** — OIDC/SAML federation → **Keycloak** by default (BYO IdP, sovereign).
- **Authz / capability / delegation** — **Biscuit** tokens minted from the identity;
  **session join links = attenuated Biscuits** ("participant, session X, expires H+2,
  may answer"). Delegation = Biscuit attenuation.

### Real-time & scale (horizontal)

- WebSocket per participant; **Redis pub/sub** for live event fanout across instances
  (low-latency, fire-and-forget). **Pulsar** reserved for durable heavy jobs (TTS,
  ingestion). Session state / presence / rate-limits in **Materia KV / Redis**.
- 200/session is trivial for one tokio instance; the backplane makes multi-session /
  multi-instance real. Wired from the tracer-bullet so load tests are meaningful.

### Data & AI (sovereign, BYO/BYOK)

- **PostgreSQL + pgvector** (RAG: hybrid vector + FTS). **Cellar** (S3) for uploads +
  generated artifacts. **Clever Cloud** region `par`.
- AI behind an **OpenAI-compatible** abstraction → **Clever AI** default; **BYOK**
  (Clever AI sous contrat, ou runtime local loopback). Embeddings + LLM + TTS.
- **Cost discipline (mandatory): precompute-once, serve-to-many** + per-session
  rate-limits. Never 200 concurrent LLM streams; shared artifacts are generated once
  (idempotent, content-addressed) and served from Cellar/cache.

## Algorithmics

- **Ingestion**: extract (PDF/OCR) → structural segmentation → chunking (recursive/
  semantic, overlap, source metadata) → batch embeddings → upsert pgvector.
- **Retrieval (RAG)**: embed query → hybrid search (pgvector + Postgres FTS) → rerank →
  bounded context → grounded generation with citations (chunk_id → source offsets) →
  refusal below a confidence threshold.
- **Generators** (each grounded): summary (map-reduce); quiz/flashcards (constrained
  question-generation + dedup); mind map (concept graph). Solo formats deferred.
- **Live quiz**: state machine `lobby → question(timer) → reveal → leaderboard → next`;
  answers aggregated in Materia KV; scoring (speed + accuracy); fanout via Redis.

## Decomposition & sequencing (risk-first)

Platform = 4 sub-projects, each its own spec → plan → build:

- **P1** — Ingestion + RAG foundation.
- **P2** — Studio generators (grounded quiz/flashcards/mindmap/summary).
- **P3** — Live collaborative session (the 200-user core, the differentiator).
- **P4** — Self-host / BYO / admin / sovereignty (auth, quotas, audit, RGPD).

**Build order = risk-first, not dependency-first.** The RAG (P1) is low-risk/solved;
the live engine (P3) is the high-risk, high-uncertainty differentiator and is
corpus-agnostic. So **P3 tracer-bullet first** against a fixture corpus, then plug in
P1's real RAG.

### Tracer-bullet (first implementation milestone)

A thin vertical slice, load-tested:

> host creates a session → gets a **Biscuit-attenuated join link** → N participants
> join (web/wasm) → host pushes **one** question (fixture corpus) → **answers aggregated
> via Redis pub/sub** → **real-time leaderboard** → sustained at **200 then 500
> concurrent** participants across **≥2 instances**.

Proves the spine: WebSocket + backplane + shared state + scale + Biscuit join links.
Everything else (real RAG, 20+ question types, Studio) grafts onto a proven spine.

## Sovereignty & compliance

- No US hyperscaler; Clever Cloud only; all storage EU (`par`).
- Dependencies: permissive licenses (MIT/Apache/MPL); enforce with `cargo deny`.
- RGPD: data deletion workflow, EU residency, no PII in logs. Audit trail (DORA).
- Biscuit (not JWT) for authz; secrets in env vars only; TLS everywhere.

## Open items

- Exact fixture-corpus shape for the tracer-bullet.
- Redis pub/sub vs a Pulsar topic for live fanout (leaning Redis; revisit under load).
- UniFFI vs hand-written FFI for the native bindings (post-tracer-bullet).
