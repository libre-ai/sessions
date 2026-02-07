# P3 Tracer-Bullet — Plan (architecture-verified)

> Build-ready plan for the live-session engine. Architecture **adversarially verified**
> by a multi-agent workflow: all naive designs fail at 500 concurrent, so we ship a
> single-instance spine **behind retrofit seams** and scale in later slices.

## Verified decisions

- **TB-1 = single-instance, lock-free spine.** Prove 200 concurrent on one Clever
  instance first; do NOT pre-build horizontal scale.
- **Seams as traits** so TB-2 swaps in distributed impls without a rewrite:
  `SessionStore` (in-memory `DashMap` → Postgres), `Fanout` (tokio `broadcast` →
  Redis pub/sub), `RateLimiter` (in-memory `AtomicU32` → KV).
- **Live fanout = Redis** (Clever managed). **Session state = PostgreSQL first**
  (mature, ACID, ~2500 inserts/session is trivial). **Materia KV is beta / unknown
  limits — measure in TB-2, only adopt if Postgres bottlenecks.** Pulsar = durable
  post-quiz jobs (P2+).
- **Fanout** = `tokio::sync::broadcast::channel(2048)` per session (not 1024 —
  saturation observed at 500). Add **sequence numbers + client dedup** in TB-2.
- **Crash resilience**: TB-1 accepts session loss (1h sessions); TB-2 adds a
  Postgres write-ahead answer log; replay protection via Biscuit nonce in TB-2/3.

## TB-1 design

**State** (server, in-process, behind `SessionStore`):
`Arc<DashMap<SessionId, Arc<SessionState>>>`; per-session `broadcast::Sender<ServerMessage>`.

**Wire protocol** (`presto-core`, serde JSON, one message per frame):

- `ClientMessage`: `Join`, `SubmitAnswer { question_id, choice, elapsed_ms }`, `Ping`.
- `ServerMessage`: `QuestionOpened { id, text, choices, timer_sec }`, `AnswerReceived { participant_id }`, `AnswersRevealed { correct_choice, leaderboard, heatmap }`, `Error`, `Pong`.

**Auth**: host mints an **attenuated Biscuit** per join link `{session_id, participant_id, capability:[answer], exp:+2h}`; tower middleware validates (session match + expiry) before WS upgrade. Host link carries `capability:[host]`.

**Scoring** (on reveal): `correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0`; leaderboard sorted desc.

**Fixture corpus**: 5 hardcoded questions (no RAG yet).

## Slices

| Slice     | Adds                                                                                                                                     |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| **TB-1a** | `presto-core::protocol` (wire types) + pure `Session` engine (join/push/submit/reveal/scoring) — fully unit-tested. No async/WS/Biscuit. |
| **TB-1b** | Trait seams (`SessionStore`/`Fanout`/`RateLimiter`) + in-memory impls.                                                                   |
| **TB-1c** | axum WS handler + registry + broadcast fanout; integration test (host + N participants).                                                 |
| **TB-1d** | Biscuit attenuated join-link middleware (host vs participant capability).                                                                |
| **TB-1e** | Fixture push API + k6 load test (200 concurrent, p99 < 200ms, 0 loss).                                                                   |
| **TB-2**  | Redis fanout + Postgres state + seq numbers (2–3 instances, load 300).                                                                   |
| **TB-3**  | WAL + crash recovery + Biscuit nonce replay protection (chaos test).                                                                     |
| **TB-4**  | Web/wasm participant client + Keycloak + real RAG questions.                                                                             |
| **TB-5**  | Load test 500 on 2–4 instances + chaos + tuning (p99 < 150ms).                                                                           |

## Top risks (from adversarial verify)

1. **Materia KV beta limits** → don't architect on it; Postgres-first; measure Materia in TB-2 before adopting.
2. **Broadcast saturation at 500** → buffer 2048 + message tiering + seq numbers (TB-2); load-test 0-loss at 500.
3. **Instance crash orphans participants** → TB-1 documents the limitation; TB-2 Postgres WAL + recovery within 5s.
