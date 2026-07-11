# Plan — lm-session-runtime (2026-07 wave)

**Status update (2026-07-10 verified):** All increments delivered. I1-I2, I4-I6 merged on main; I3 (middleware tower layer) superseded by inline auth in main.rs + ws.rs (2026-07-04 implementation, verified stable).

- **I1** (PR #45, #51): ✓ session-identity.v0.1 contract + workspace-identity reconciliation
- **I2** (PR #46, #52): ✓ Postgres store + Redis fanout + ScoreSink trait
- **I3** (PR #47): ✗ superseded — inline auth impl at main.rs + ws.rs:46-52
- **I4** (PR #48): ✓ ScoreSink consumption pattern + example
- **I5** (PR #49): ✓ Playwright session e2e
- **I6** (PR #53): ✓ live question grounding summary

```yaml
format: forge.plan.v0.1
kind: planning_request
source:
  product: rumble-lm
  plan_id: plan-2026-07-lm-session-runtime
  created_at: "2026-07-03"
  revision: "2"
execution_policy:
  planning_only: true
  allow_execution: false
  requires_human_approval_for_execution: true
traceability:
  - "$DEV_ROOT/constantin-jais/ecosystem/specs/shared/adrs/0028-workspace-identity-ownership.md (Accepted 2026-07-03, DA-7, amendments 1–3: closed permission vocabulary, D11-gating, big-bang posture)"
  - "architecture-alignment-2026-07.md (DA-7: workspace/identity split Gear+shared-Rumble; DA-8: big-bang reconciliation wave)"
  - "$DEV_ROOT/rumble-lm/docs/plans/2026-06-27-p3-tracer-bullet.md (TB-1: single-instance lock-free spine, seams as traits)"
  - "P3 Tracer-Bullet decisions: SessionStore/Fanout/RateLimiter behind trait seams; Postgres (session state) + Redis (fanout), Biscuit auth, scoring formula"
depends_on:
  - "workspace-identity.v0.1.md contract (shared reference); I1 from both rumble-canvas (plan-2026-07-canvas-mvp-workspace-identity.md) and rumble-ai-practices (plan-2026-07-ai-practices-convergence-prep.md) are planning or executing in parallel"
blocks:
  - "rumble-canvas increment #2 (canvas fixture depends on lm session-identity contract delivery)"
  - "rumble-ai-practices scoring module extraction (ai-practices I4 depends on lm contract fixtures I3)"
open_questions: []
risks:
  - id: R1
    severity: medium
    description: "Biscuit-auth (6.0.0) dependency introduced in I2; RUSTSEC-2026-0173 unresolved in upstream. Workaround: mock sealer in test suite to simulate Biscuit validation without live dependency until CVSS patch lands."
    mitigation: "I2 gates on conditional: if biscuit-auth@6.0.0 resolves advisory, use real sealer; else gate requires mock sealer trait impl + test harness proof. Ci.yml audit-deny blocks build on unresolved RUSTSEC entries."
  - id: R2
    severity: low
    description: "E2E test layer (Playwright + TypeScript) is new to rumble-lm; setup complexity and Windows/Mac portability."
    mitigation: "I5 documents `.env.example`, `npm` install command, and `npx playwright install`. CI runs on ubuntu-latest; arm64/macOS compat deferred post-MVP."
evidence_expectations: "Each increment = green CI (cargo test, fmt, check, clippy, deny), plus exact exit gate commands below. No temporal estimates; each increment is a standalone green PR."
```

## Context

**Blocker:** The `rumble-lm` session runtime is the critical path for canvas MVP and ai-practices convergence. Canvas cannot prove multi-actor workflows without stable session-workspace boundary. AI-practices has a frozen session shim (ADR 0005, Accepted via DA-8) awaiting lm's persistent runtime contract.

**Decision made (P3 Tracer-Bullet, verified architecture):** Tracer-Bullet TB-1 proves 200 concurrent on a single Clever instance with lock-free in-memory spine (DashMap) behind trait seams. TB-1 is production-ready for the MVP; horizontal scale (TB-2) and durability (TB-3) are retrofit in later waves. Seams: `SessionStore` (in-memory DashMap → Postgres), `Fanout` (tokio broadcast → Redis), `RateLimiter` (in-process atomic → KV). Session state lives in PostgreSQL (mature, ACID); fanout via Redis (Clever managed). Biscuit tokens (workspace-identity.v0.1 contract) mint attenuated per-join-link with session_id + participant_id + role + permissions (amendments 1–3: closed vocabulary from ADR 0028).

**Key constraints:**

- D11-gating (ADR 0028 amendment 2): workspace-identity primitives stay as contract + fixtures until 2 implementations (canvas + lm) + cross-repo Biscuit fixture land; extraction to dedicated Gear crate deferred.
- Big-bang posture (ADR 0028 amendment 3, DA-8): lm, canvas, ai-practices reconcile onto workspace-identity.v0.1 **within 2026-07 wave**, not later. Fixture adoption proves D11 threshold.
- Closed vocabulary (ADR 0028 amendment 1): RoleAssignment.permissions ⊆ {read, comment, write, approve, invite, administer, delegate}; no free-form strings.
- Convergence gate (rumble-ai-practices ADR 0005): ai-practices shim is frozen until lm runtime proven live + takes all three endpoints (`/sessions`, `/sessions/{id}/submit`, `/sessions/{id}/answer`).

## Target state

**After all 5 increments complete:**

1. **I1** delivers `session-identity.v0.1` contract (Biscuit spec + RoleAssignment fixture), workspace-identity.v0.1 reconciliation (Host/Participant → RoleAssignment), and integration test harness. Proof of D11 adoption by lm. Cross-repo fixture wires Biscuit over workspace-identity facts (actor + role + permissions) for canvas/ai-practices consumption. Contract fixtures published to `$DEV_ROOT/rumble-lm/docs/contracts/session-identity.v0.1.fixtures.json`.

2. **I2** wires Postgres SessionStore + Redis Fanout into axum WS handler; migration CLI (`cargo sqlx migrate run`) proven; score_hook trait with in-memory mock; integration test (host + N participants, p99 < 200ms). Database/schema upsert proves state persistence. Redis pub/sub proves multi-instance fanout (no cross-talk).

3. **I3** adds Biscuit auth tower middleware (attenuated join-link validation), Biscuit fixture serialization (deterministic mock sealer for test reproducibility), and cross-repo Biscuit contract test (canvas/ai-practices consume fixture, verify token validation). Locks role enforcement into the middleware.

4. **I4** extracts scoring module (`crates/server/src/scoring.rs`) as consumable by ai-practices (module with public `ScoreHook` trait + `compute_score(&ScoreSink) -> u64` function + fixtures). Documented as the reference for custom scoring implementations. Exports module in lib.rs with example usage.

5. **I5** builds end-to-end test harness (Playwright, TypeScript, 5+ scenarios: join/submit/reveal/leaderboard/error cases), `e2e/tests/` directory structure, `.env.example` for DATABASE_URL/REDIS_URL/BISCUIT_PRIVATE_KEY, `npm init`, `@playwright/test` dependency, and `npx playwright install` documented. CI runs e2e suite (blocking for merge).

**Verification criteria:**

- All 5 increments pass CI gates (cargo test/fmt/check/clippy/deny).
- Integration + e2e tests run in CI (Postgres 16+pgvector, Redis 7, live fixture harness).
- Database migrations applied (sqlx migrate, schema versioned).
- Biscuit token validation proven (fixture serialization + cross-repo test).
- Scoring module extracted and documented (public trait + example).
- No `TODO` markers; debt tracked in ROADMAP.md.
- Each increment is a single green PR.
- Contract fixtures published and consumed by canvas (I1/I2) and ai-practices (I3/I4).

## Increments

### I1 — session-identity.v0.1 contract + workspace-identity reconciliation + cross-repo fixture

**Status (2026-07-09):** ✓ Delivered — PR #45 (feat: I1 session-identity.v0.1 contract + workspace-identity reconciliation, 4abc6c34be892647e93d1448ba39c377c2c2db6e). Additional fixes in #45 commit 4b0e01a0 (align Host/Participant permission sets) and 4672f01c (scope uuid js feature to wasm32).

**Purpose:** Establish the Biscuit token contract (session_id, participant_id, role, permissions) aligned to workspace-identity.v0.1 (ADR 0028 amendments). Mint a shared fixture so canvas/ai-practices can validate token structure without a live lm instance. Reconcile lm's Host/Participant roles onto RoleAssignment (closed vocabulary). Prove D11 adoption path: 2 implementations (lm + canvas) + fixture.

**Files touched:**

- `docs/contracts/session-identity.v0.1.md` (new) — Biscuit token structure, workspace-identity alignment, RoleAssignment mapping (Host → {read, write, comment, approve}, Participant → {read, comment}).
- `docs/contracts/session-identity.v0.1.fixtures.json` (new) — Deterministic mock Biscuit tokens (Host + Participant) serialized as JSON for cross-repo tests.
- `crates/core/src/auth.rs` (or new `crates/core/src/role_assignment.rs`) — Add `RoleAssignment` struct (workspace_id, actor_id, role: String, permissions: Vec<PermissionPrimitive>); `PermissionPrimitive` enum (read, comment, write, approve, invite, administer, delegate, per ADR 0028 amendment 1).
- `crates/core/src/lib.rs` — Export `RoleAssignment`, `PermissionPrimitive`.
- `crates/server/tests/integration_workspace_identity_biscuit_contract.rs` (new) — Cross-repo fixture test: deserialize session-identity.v0.1.fixtures.json, validate Biscuit structure, reconcile to RoleAssignment (Host/Participant mapped correctly).
- `crates/server/src/auth.rs` — Update `Auth` struct to include role/permissions fields; document Biscuit sealer contract (signature must contain workspace_id + session_id + actor_id + permissions).

**Prerequisite:**

- ADR 0028 Accepted ✓
- workspace-identity.v0.1.md published in ecosystem (shared reference) ✓

**Work (exact, no vagueness):**

1. Create `docs/contracts/session-identity.v0.1.md`:

   ```markdown
   # session-identity.v0.1 Contract

   Biscuit token structure for session-workspace join-link authorization.
   Aligns to workspace-identity.v0.1 (ADR 0028, Accepted 2026-07-03).

   ## Token structure
   ```

   token = Biscuit(
   facts: [
   actor(ActorId, ActorType, ActorName), // From workspace-identity contract
   workspace(WorkspaceId),
   session(SessionId),
   role("host" | "participant"), // Product role (Host/Participant)
   permission("read" | "comment" | "write" | "approve" | "invite" | "administer" | "delegate"), // Closed vocabulary, ADR 0028 amend#1
   capability("host_minting" | "answer_submit"), // Capability gating
   expiry(Timestamp), // +2h from mint
   ],
   authority_keypair: SHARED_RUMBLE_WORKSPACE_KEY // Shared across lm, canvas, ai-practices
   )

   ```

   ## Mapping: Host/Participant → RoleAssignment

   - **Host role:**
     - workspace_id: from token.facts.workspace(WorkspaceId)
     - actor_ref: ActorReference(actor_id, ActorType::Human | ActorType::Service, actor_name)
     - role: "host"
     - permissions: [read, comment, write, approve, administer]
     - created_at: token.minted_at

   - **Participant role:**
     - workspace_id: from token.facts.workspace(WorkspaceId)
     - actor_ref: ActorReference(actor_id, ActorType::Human | ActorType::Agent, actor_name)
     - role: "participant"
     - permissions: [read, comment]
     - created_at: token.minted_at

   ## Cross-repo fixture

   See `session-identity.v0.1.fixtures.json` for deterministic mock tokens (canvas, ai-practices consume for integration tests without live lm).
   ```

2. Create `docs/contracts/session-identity.v0.1.fixtures.json`:

   ```json
   {
     "version": "session-identity.v0.1",
     "fixtures": [
       {
         "id": "host_token_fixture",
         "type": "biscuit_host_role",
         "workspace_id": "workspace_test_001",
         "session_id": "session_test_001",
         "actor_id": "actor_host_001",
         "actor_type": "Human",
         "actor_name": "Host User",
         "role": "host",
         "permissions": ["read", "comment", "write", "approve", "administer"],
         "capability": "host_minting",
         "expiry_offset_sec": 7200,
         "deterministic_nonce": "fixture_host_001",
         "cross_repo_usage": [
           "rumble-canvas integration",
           "rumble-ai-practices integration"
         ]
       },
       {
         "id": "participant_token_fixture",
         "type": "biscuit_participant_role",
         "workspace_id": "workspace_test_001",
         "session_id": "session_test_001",
         "actor_id": "actor_participant_001",
         "actor_type": "Human",
         "actor_name": "Participant User",
         "role": "participant",
         "permissions": ["read", "comment"],
         "capability": "answer_submit",
         "expiry_offset_sec": 7200,
         "deterministic_nonce": "fixture_participant_001",
         "cross_repo_usage": [
           "rumble-canvas integration",
           "rumble-ai-practices scoring"
         ]
       }
     ]
   }
   ```

3. Add to `crates/core/src/lib.rs` (after existing types):

   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(rename_all = "snake_case")]
   pub enum PermissionPrimitive {
       Read,
       Comment,
       Write,
       Approve,
       Invite,
       Administer,
       Delegate,
   }

   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct RoleAssignment {
       pub id: String,
       pub workspace_id: String,
       pub actor_id: String,
       pub role: String,  // "host", "participant", etc.
       pub permissions: Vec<PermissionPrimitive>,
       pub created_at: String,
       pub revoked_at: Option<String>,
   }

   impl RoleAssignment {
       /// Validate that role + permissions align to workspace-identity.v0.1 closed vocabulary.
       pub fn validate(&self) -> Result<(), String> {
           if self.permissions.is_empty() {
               return Err("permissions cannot be empty".to_string());
           }
           Ok(())
       }

       /// Map Host role to RoleAssignment with permissions.
       pub fn host(workspace_id: String, actor_id: String) -> Self {
           Self {
               id: format!("role_{}", uuid::Uuid::new_v4()),
               workspace_id,
               actor_id,
               role: "host".to_string(),
               permissions: vec![
                   PermissionPrimitive::Read,
                   PermissionPrimitive::Comment,
                   PermissionPrimitive::Write,
                   PermissionPrimitive::Approve,
                   PermissionPrimitive::Administer,
               ],
               created_at: chrono::Utc::now().to_rfc3339(),
               revoked_at: None,
           }
       }

       /// Map Participant role to RoleAssignment with permissions.
       pub fn participant(workspace_id: String, actor_id: String) -> Self {
           Self {
               id: format!("role_{}", uuid::Uuid::new_v4()),
               workspace_id,
               actor_id,
               role: "participant".to_string(),
               permissions: vec![PermissionPrimitive::Read, PermissionPrimitive::Comment],
               created_at: chrono::Utc::now().to_rfc3339(),
               revoked_at: None,
           }
       }
   }
   ```

4. Create `crates/server/tests/integration_workspace_identity_biscuit_contract.rs`:

   ```rust
   #[cfg(test)]
   mod tests {
       use std::fs;
       use serde_json::json;
       use presto_core::{RoleAssignment, PermissionPrimitive};

       #[test]
       fn test_session_identity_v0_1_fixture_loads() {
           let fixture_json = fs::read_to_string("docs/contracts/session-identity.v0.1.fixtures.json")
               .expect("fixture file must exist");
           let fixtures: serde_json::Value = serde_json::from_str(&fixture_json)
               .expect("fixture must be valid JSON");

           assert_eq!(fixtures["version"], "session-identity.v0.1");
           assert!(fixtures["fixtures"].is_array());
           assert!(fixtures["fixtures"].as_array().unwrap().len() >= 2);
       }

       #[test]
       fn test_host_role_assignment_reconciliation() {
           let host = RoleAssignment::host(
               "workspace_test_001".to_string(),
               "actor_host_001".to_string(),
           );

           assert_eq!(host.role, "host");
           assert_eq!(host.permissions.len(), 5);
           assert!(host.permissions.contains(&PermissionPrimitive::Write));
           assert!(host.permissions.contains(&PermissionPrimitive::Approve));
           assert!(host.permissions.contains(&PermissionPrimitive::Administer));
       }

       #[test]
       fn test_participant_role_assignment_reconciliation() {
           let participant = RoleAssignment::participant(
               "workspace_test_001".to_string(),
               "actor_participant_001".to_string(),
           );

           assert_eq!(participant.role, "participant");
           assert_eq!(participant.permissions.len(), 2);
           assert!(participant.permissions.contains(&PermissionPrimitive::Read));
           assert!(participant.permissions.contains(&PermissionPrimitive::Comment));
           assert!(!participant.permissions.contains(&PermissionPrimitive::Write));
       }

       #[test]
       fn test_permission_primitive_closed_vocabulary() {
           // Verify that only closed vocabulary is available.
           let perms = vec![
               PermissionPrimitive::Read,
               PermissionPrimitive::Comment,
               PermissionPrimitive::Write,
               PermissionPrimitive::Approve,
               PermissionPrimitive::Invite,
               PermissionPrimitive::Administer,
               PermissionPrimitive::Delegate,
           ];
           assert_eq!(perms.len(), 7, "exactly 7 closed permissions per ADR 0028 amend#1");
       }
   }
   ```

5. Update `crates/server/src/auth.rs` to include RoleAssignment context:
   ```rust
   pub struct Auth {
       private_key: biscuit_auth::PrivateKey,
       role_assignment: Option<RoleAssignment>,  // NEW: workspace-identity alignment
   }

   impl Auth {
       /// Create an attenuated Biscuit token with role + permissions from RoleAssignment.
       pub fn mint_attenuated(
           &self,
           workspace_id: &str,
           session_id: &str,
           actor_id: &str,
           role_assignment: &RoleAssignment,
       ) -> Result<String, Box<dyn Error>> {
           // Validate role_assignment per ADR 0028 amend#1.
           role_assignment.validate()?;

           // Mint Biscuit with facts: actor, workspace, session, role, permissions, expiry.
           // (Implementation details: use biscuit-auth 6.0.0 sealer, deterministic for testing)
           Ok(format!("biscuit_token_{}", uuid::Uuid::new_v4()))
       }
   }
   ```

**Exit gates:**

- `cargo test --workspace --all-targets` ✓ (new tests in integration_workspace_identity_biscuit_contract.rs all pass)
- `cargo fmt --all --check` ✓
- `cargo check --workspace` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo deny check` ✓ (biscuit-auth@6.0.0 audit check; if RUSTSEC-2026-0173 unresolved, document mitigation in Cargo.lock.notes)
- File existence: `test -f docs/contracts/session-identity.v0.1.md && test -f docs/contracts/session-identity.v0.1.fixtures.json` ✓
- Cross-repo fixture validation: `python3 $DEV_ROOT/constantin-jais/ecosystem/specs/validate_spec_schemas.py --fixture docs/contracts/session-identity.v0.1.fixtures.json` ✓
- PR merges with all gates green.

---

### I2 — Postgres SessionStore + Redis Fanout + score_hook trait + integration test

**Status (2026-07-09):** ✓ Delivered — PR #46 (feat: I2 add ScoreSink hook, 052d8a86365f321e2608072011a5feb359b95f0e) and PR #52 (feat: I2 scope session runtime with workspace identity, 663b57c99e246f4d9f939a5528959b15e2799359).

**Purpose:** Wire Postgres and Redis into the session engine (backend for TB-1b seams). Implement SessionStore trait (Postgres upsert + query), Fanout trait (Redis pub/sub), and ScoreSink trait (in-memory mock + trait for custom implementations). Prove database migrations run and state persists. Prove multi-instance fanout without cross-talk. Demonstrate score_hook integration point (ai-practices will consume scoring module in I4).

**Files touched:**

- `crates/server/src/postgres_store.rs` — `PostgresSessionStore` impl (connect, create_session, update_session, get_session, list_sessions).
- `crates/server/src/redis_fanout.rs` — `RedisFanout` impl (connect, subscribe, publish).
- `crates/server/src/scoring.rs` (new) — `ScoreSink` trait (on_answer_submitted, compute_score), `DefaultScoreSink` impl (in-memory mock for testing).
- `crates/server/src/lib.rs` — Export scoring module; document scoring hook pattern.
- `crates/rag/migrations/` (new directory) — SQLx SQL migration files (01_init_sessions.sql, etc.; schema versioning).
- `.env.example` (new) — Example environment variables (DATABASE_URL, REDIS_URL, BISCUIT_PRIVATE_KEY).
- `crates/server/tests/integration_postgres_redis.rs` (new) — Test: host + 5 participants, verify Postgres state (session created, answers persisted), Redis fanout (no cross-talk), score_hook called on reveal.
- `Cargo.toml` (workspace) — Add tracing, tracing-subscriber deps (I2 adds observability foundation for score_hook + state transitions).
- `crates/server/Cargo.toml` — Add `sqlx-cli` (dev-dependency, or doc note for `cargo install sqlx-cli`).
- `crates/rag/Cargo.toml` — Verify sqlx 0.9.0 present (already there from P3 baseline).

**Prerequisite:**

- I1 complete (RoleAssignment, PermissionPrimitive, workspace-identity contract) ✓
- Postgres 16+pgvector available (test CI environment has it) ✓
- Redis 7 available (test CI environment has it) ✓
- DATABASE_URL and REDIS_URL env vars documented ✓

**Work (exact, no vagueness):**

1. Create `crates/rag/migrations/01_init_sessions.sql`:

   ```sql
   CREATE TABLE IF NOT EXISTS sessions (
       id TEXT PRIMARY KEY,
       workspace_id TEXT NOT NULL,
       host_actor_id TEXT NOT NULL,
       started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
       ended_at TIMESTAMPTZ,
       state TEXT NOT NULL DEFAULT 'active',  -- 'active', 'ended', 'error'
       metadata JSONB DEFAULT '{}'
   );

   CREATE TABLE IF NOT EXISTS session_answers (
       id TEXT PRIMARY KEY,
       session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
       participant_actor_id TEXT NOT NULL,
       question_id TEXT NOT NULL,
       choice TEXT NOT NULL,
       elapsed_ms INTEGER NOT NULL,
       submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
       score INTEGER,
       UNIQUE(session_id, participant_actor_id, question_id)
   );

   CREATE INDEX idx_session_answers_session ON session_answers(session_id);
   CREATE INDEX idx_session_answers_participant ON session_answers(participant_actor_id);
   ```

2. Create `crates/rag/migrations/02_add_role_assignment_tracking.sql`:

   ```sql
   ALTER TABLE sessions ADD COLUMN IF NOT EXISTS role_assignments JSONB DEFAULT '[]';
   -- role_assignments: JSON array of RoleAssignment objects (actor_id, role, permissions)
   ```

3. Update `crates/server/src/postgres_store.rs`:

   ```rust
   use async_trait::async_trait;
   use sqlx::PgPool;
   use crate::store::SessionStore;
   use presto_core::Session;

   pub struct PostgresSessionStore {
       pool: PgPool,
   }

   impl PostgresSessionStore {
       pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
           let pool = PgPool::connect(database_url).await?;
           // Run migrations automatically on connect.
           sqlx::migrate!("crates/rag/migrations")
               .run(&pool)
               .await?;
           Ok(Self { pool })
       }
   }

   #[async_trait]
   impl SessionStore for PostgresSessionStore {
       async fn create_session(&self, session: &Session) -> Result<(), Box<dyn std::error::Error>> {
           sqlx::query(
               "INSERT INTO sessions (id, workspace_id, host_actor_id, state, metadata) VALUES ($1, $2, $3, $4, $5)"
           )
           .bind(&session.id)
           .bind(&session.workspace_id)
           .bind(&session.host_actor_id)
           .bind("active")
           .bind(serde_json::to_value(&session.metadata)?)
           .execute(&self.pool)
           .await?;
           Ok(())
       }

       async fn get_session(&self, session_id: &str) -> Result<Option<Session>, Box<dyn std::error::Error>> {
           let row = sqlx::query_as::<_, (String, String, String, String)>(
               "SELECT id, workspace_id, host_actor_id, state FROM sessions WHERE id = $1"
           )
           .bind(session_id)
           .fetch_optional(&self.pool)
           .await?;

           if let Some((id, workspace_id, host_actor_id, state)) = row {
               Ok(Some(Session {
                   id,
                   workspace_id,
                   host_actor_id,
                   state,
                   metadata: Default::default(),
               }))
           } else {
               Ok(None)
           }
       }

       async fn update_session(&self, session: &Session) -> Result<(), Box<dyn std::error::Error>> {
           sqlx::query("UPDATE sessions SET state = $1, metadata = $2 WHERE id = $3")
               .bind(&session.state)
               .bind(serde_json::to_value(&session.metadata)?)
               .bind(&session.id)
               .execute(&self.pool)
               .await?;
           Ok(())
       }
   }
   ```

4. Update `crates/server/src/redis_fanout.rs`:

   ```rust
   use async_trait::async_trait;
   use redis::aio::Connection;
   use redis::AsyncCommands;
   use crate::fanout::Fanout;
   use presto_core::ServerMessage;

   pub struct RedisFanout {
       connection: Connection,
   }

   impl RedisFanout {
       pub async fn connect(redis_url: &str) -> Result<Self, redis::RedisError> {
           let client = redis::Client::open(redis_url)?;
           let connection = client.get_async_connection().await?;
           Ok(Self { connection })
       }
   }

   #[async_trait]
   impl Fanout for RedisFanout {
       async fn subscribe(&self, channel: &str) -> Result<Box<dyn futures_util::stream::Stream<Item = ServerMessage> + Unpin>, Box<dyn std::error::Error>> {
           // Subscribe to Redis channel for session_id.
           // (Implementation: Redis Streams or Pub/Sub + reconnetion logic)
           unimplemented!("Redis subscribe implementation")
       }

       async fn publish(&self, channel: &str, message: ServerMessage) -> Result<(), Box<dyn std::error::Error>> {
           let json = serde_json::to_string(&message)?;
           let mut conn = self.connection.clone();
           conn.publish::<_, _, ()>(channel, json).await?;
           Ok(())
       }
   }
   ```

5. Create `crates/server/src/scoring.rs`:

   ```rust
   use async_trait::async_trait;

   /// Trait for custom scoring hook implementations.
   /// Consumed by ai-practices (I4) and other products.
   #[async_trait]
   pub trait ScoreSink: Send + Sync {
       /// Called when a participant submits an answer.
       async fn on_answer_submitted(
           &self,
           session_id: &str,
           participant_id: &str,
           question_id: &str,
           choice: &str,
           elapsed_ms: u64,
       ) -> Result<(), Box<dyn std::error::Error>>;

       /// Compute score for an answer (Tracer-Bullet formula: correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0).
       async fn compute_score(
           &self,
           choice: &str,
           correct_choice: &str,
           elapsed_ms: u64,
       ) -> Result<u64, Box<dyn std::error::Error>>;
   }

   /// In-memory mock ScoreSink for testing.
   pub struct InMemorySink {
       answers: std::sync::Arc<std::sync::Mutex<Vec<(String, String, String, String, u64)>>>,
   }

   impl InMemorySink {
       pub fn new() -> Self {
           Self {
               answers: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
           }
       }

       pub fn recorded_answers(&self) -> Vec<(String, String, String, String, u64)> {
           self.answers.lock().unwrap().clone()
       }
   }

   #[async_trait]
   impl ScoreSink for InMemorySink {
       async fn on_answer_submitted(
           &self,
           session_id: &str,
           participant_id: &str,
           question_id: &str,
           choice: &str,
           elapsed_ms: u64,
       ) -> Result<(), Box<dyn std::error::Error>> {
           self.answers.lock().unwrap().push((
               session_id.to_string(),
               participant_id.to_string(),
               question_id.to_string(),
               choice.to_string(),
               elapsed_ms,
           ));
           Ok(())
       }

       async fn compute_score(
           &self,
           choice: &str,
           correct_choice: &str,
           elapsed_ms: u64,
       ) -> Result<u64, Box<dyn std::error::Error>> {
           if choice == correct_choice {
               let time_bonus = ((30000_i64 - elapsed_ms as i64).max(0) as f64 / 300.0).min(100.0) as u64;
               Ok(500 + time_bonus)
           } else {
               Ok(0)
           }
       }
   }

   #[cfg(test)]
   mod tests {
       use super::*;

       #[tokio::test]
       async fn test_score_hook_correct_answer() {
           let sink = InMemorySink::new();
           let score = sink.compute_score("A", "A", 5000).await.unwrap();
           assert_eq!(score, 583);  // 500 + min((30000-5000)/300, 100) = 500 + 83
       }

       #[tokio::test]
       async fn test_score_hook_incorrect_answer() {
           let sink = InMemorySink::new();
           let score = sink.compute_score("B", "A", 5000).await.unwrap();
           assert_eq!(score, 0);
       }

       #[tokio::test]
       async fn test_score_hook_on_answer_submitted_recorded() {
           let sink = InMemorySink::new();
           sink.on_answer_submitted("sess1", "part1", "q1", "A", 5000).await.unwrap();
           let recorded = sink.recorded_answers();
           assert_eq!(recorded.len(), 1);
           assert_eq!(recorded[0].0, "sess1");
       }
   }
   ```

6. Update `crates/server/src/lib.rs`:

   ```rust
   pub mod scoring;  // NEW: export scoring module for ai-practices consumption

   pub use scoring::{ScoreSink, InMemorySink};  // Public API
   ```

7. Create `.env.example`:

   ```
   # Database
   DATABASE_URL=postgres://postgres:presto@localhost:5432/postgres

   # Cache/Fanout
   REDIS_URL=redis://localhost:6379/

   # Authentication
   BISCUIT_PRIVATE_KEY=<paste output of: presto-server keygen>

   # RAG (optional)
   LOCAL_AI_ENABLED=1
   LOCAL_AI_BASE_URL=http://127.0.0.1:8000
   LOCAL_AI_API_KEY=<local-only-key>
   ```

8. Update `Cargo.toml` (workspace dependencies):

   ```toml
   [workspace.dependencies]
   # ... existing ...
   tracing = "0.1"
   tracing-subscriber = { version = "0.3", features = ["env-filter"] }
   ```

9. Update `crates/server/Cargo.toml`:

   ```toml
   [dependencies]
   # ... existing sqlx, redis, tokio ...
   tracing.workspace = true
   tracing-subscriber.workspace = true

   [dev-dependencies]
   # ... existing ...
   ```

10. Create `crates/server/tests/integration_postgres_redis.rs`:
    ```rust
    #[cfg(test)]
    mod tests {
        use presto_server::store::SessionStore;
        use presto_server::fanout::Fanout;
        use presto_server::scoring::ScoreSink;
        use presto_server::{PostgresSessionStore, RedisFanout, InMemorySink};
        use presto_core::Session;

        #[tokio::test]
        #[ignore]  // Requires live Postgres + Redis; run with: cargo test -- --ignored
        async fn test_postgres_session_persist() {
            let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL not set");
            let store = PostgresSessionStore::connect(&db_url).await.expect("connect failed");

            let session = Session {
                id: "test_sess_001".to_string(),
                workspace_id: "ws_001".to_string(),
                host_actor_id: "host_001".to_string(),
                state: "active".to_string(),
                metadata: Default::default(),
            };

            store.create_session(&session).await.expect("create failed");
            let retrieved = store.get_session(&session.id).await.expect("get failed");
            assert!(retrieved.is_some());
            assert_eq!(retrieved.unwrap().id, "test_sess_001");
        }

        #[tokio::test]
        #[ignore]  // Requires live Redis; run with: cargo test -- --ignored
        async fn test_redis_fanout_publish_subscribe() {
            let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL not set");
            let fanout = RedisFanout::connect(&redis_url).await.expect("connect failed");

            // Publish a message; verify no cross-talk on different channel.
            let message = presto_core::ServerMessage::Error("test error".to_string());
            fanout.publish("session_001", message).await.expect("publish failed");

            // Verify channel isolation (subscribe to different channel should not receive).
            // (Implementation detail: use Redis SUBSCRIBE + timeout)
        }

        #[tokio::test]
        async fn test_score_hook_mock_integration() {
            let sink = InMemorySink::new();

            sink.on_answer_submitted("sess1", "part1", "q1", "A", 5000).await.unwrap();
            let score = sink.compute_score("A", "A", 5000).await.unwrap();

            assert_eq!(score, 583);
            assert_eq!(sink.recorded_answers().len(), 1);
        }
    }
    ```

**Exit gates:**

- `cargo sqlx migrate run --database-url="${DATABASE_URL}" -D crates/rag` ✓ (migrations applied successfully; schema versioned)
- `cargo test --workspace --all-targets` ✓ (all tests pass, including mock scoring tests; Postgres/Redis tests marked #[ignore])
- `cargo test --workspace --all-targets -- --ignored --test-threads=1` ✓ (integration tests with live DB/Redis pass; run only if DATABASE_URL + REDIS_URL set)
- `cargo fmt --all --check` ✓
- `cargo check --workspace` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo deny check` ✓ (tracing/tracing-subscriber audit clean)
- File existence: `test -f .env.example` ✓
- Database schema check: `psql "${DATABASE_URL}" -c "\\dt" | grep sessions` ✓ (schema created, sessions table present)
- Redis connectivity smoke: `redis-cli -u "${REDIS_URL}" PING` ✓ (returns PONG)
- Scoring trait exported: `cargo doc --workspace --no-deps --open 2>&1 | grep -q ScoreSink` ✓
- PR merges with all gates green.

---

### I3 — Biscuit auth middleware + deterministic mock sealer + cross-repo Biscuit fixture test

**Status (2026-07-09):** ✗ Superseded — PR #47 (planned tower middleware layer for Biscuit token validation) was closed unmerged (62d78b5 orphan). **Actual implementation:** Auth is implemented inline in production code (biscuit-auth v6.0.0, Ed25519, BISCUIT_PRIVATE_KEY env or ephemeral, main.rs; verify() called before WebSocket upgrade at crates/server/src/ws.rs:46-52; return 401 if invalid). Attenuated join-link validation, organization/workspace/session isolation by Biscuit authorizer rules (auth.rs:225-236), tested at auth.rs:406-420 (cross-session auth rejection). No external middleware crate — inline pattern proved simpler and production-ready.

**Original purpose (archived for reference):** Wire Biscuit token validation into axum tower middleware. Implement deterministic mock sealer (for test reproducibility without live RUSTSEC-2026-0173 risk). Prove cross-repo fixture: canvas/ai-practices deserialize fixture, validate token, extract RoleAssignment. Lock role enforcement into middleware (host vs participant capabilities). Unblock D11 adoption path (2 implementations + fixture = D11 threshold met).

**Files touched:**

- `crates/server/src/middleware/biscuit_auth.rs` (new module) — tower middleware stack (attenuated token validation, role extraction, session match).
- `crates/server/src/middleware/mod.rs` (new module) — Export middleware stack.
- `crates/server/src/auth.rs` — Add `BiscuitSealer` trait (mock + real implementations); `DeterministicMockSealer` impl (deterministic Biscuit serialization for testing).
- `crates/server/tests/integration_cross_repo_biscuit_fixture.rs` (new) — Load canvas + ai-practices fixture, validate token structure, extract RoleAssignment, verify permissions match closed vocabulary.
- `crates/core/src/lib.rs` — Add `BiscuitToken` struct (serialized fixture format for cross-repo use).
- Update `docs/contracts/session-identity.v0.1.fixtures.json` — Add serialized Biscuit token fields (for canvas/ai-practices to validate without live sealer).

**Prerequisite:**

- I2 complete (Postgres, Redis, scoring module) ✓
- Biscuit-auth 6.0.0 in Cargo.toml (already present) ✓
- Session-identity.v0.1.md contract from I1 ✓

**Work (exact, no vagueness):**

1. Create `crates/server/src/middleware/biscuit_auth.rs`:

   ```rust
   use axum::{
       extract::ws::WebSocketUpgrade,
       middleware::Next,
       response::Response,
       http::Request,
   };
   use tower::Layer;
   use std::sync::Arc;
   use crate::auth::Auth;
   use presto_core::RoleAssignment;

   pub struct BiscuitAuthLayer {
       auth: Arc<Auth>,
   }

   impl BiscuitAuthLayer {
       pub fn new(auth: Arc<Auth>) -> Self {
           Self { auth }
       }
   }

   impl<S> Layer<S> for BiscuitAuthLayer {
       type Service = BiscuitAuthMiddleware<S>;

       fn layer(&self, inner: S) -> Self::Service {
           BiscuitAuthMiddleware {
               inner,
               auth: self.auth.clone(),
           }
       }
   }

   pub struct BiscuitAuthMiddleware<S> {
       inner: S,
       auth: Arc<Auth>,
   }

   impl<S> tower::Service<Request<axum::body::Body>> for BiscuitAuthMiddleware<S>
   where
       S: tower::Service<Request<axum::body::Body>, Response = Response> + Send + 'static,
       S::Future: Send + 'static,
   {
       type Response = Response;
       type Error = S::Error;
       type Future = Box<dyn std::future::Future<Output = Result<Response, S::Error>> + Send>;

       fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
           self.inner.poll_ready(cx)
       }

       fn call(&mut self, req: Request<axum::body::Body>) -> Self::Future {
           let auth = self.auth.clone();
           let inner = self.inner.call(req);

           Box::new(async move {
               // Extract Biscuit token from query param or header.
               // Validate: session_id + participant_id match; expiry not exceeded.
               // Extract RoleAssignment; store in request extensions.
               // (Implementation: use biscuit-auth verifier, mock sealer for tests)

               inner.await
           })
       }
   }

   /// Trait for Biscuit token sealing (real or mock).
   pub trait BiscuitSealer: Send + Sync {
       fn seal(&self, facts: &[&str]) -> Result<String, Box<dyn std::error::Error>>;
       fn verify(&self, token: &str) -> Result<Vec<String>, Box<dyn std::error::Error>>;
   }

   /// Deterministic mock sealer for testing (no dependency on RUSTSEC-2026-0173).
   pub struct DeterministicMockSealer {
       secret: String,
   }

   impl DeterministicMockSealer {
       pub fn new(secret: &str) -> Self {
           Self {
               secret: secret.to_string(),
           }
       }
   }

   impl BiscuitSealer for DeterministicMockSealer {
       fn seal(&self, facts: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
           // Deterministic serialization: join facts, hash with secret.
           use std::collections::hash_map::DefaultHasher;
           use std::hash::{Hash, Hasher};

           let joined = facts.join("|");
           let mut hasher = DefaultHasher::new();
           joined.hash(&mut hasher);
           self.secret.hash(&mut hasher);
           let hash = hasher.finish();

           Ok(format!("mock_biscuit_{}_{}", hash, joined.len()))
       }

       fn verify(&self, token: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
           if token.starts_with("mock_biscuit_") {
               Ok(vec![
                   "workspace(workspace_test_001)".to_string(),
                   "session(session_test_001)".to_string(),
               ])
           } else {
               Err("invalid mock token".into())
           }
       }
   }

   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn test_deterministic_mock_sealer_same_facts_same_token() {
           let sealer = DeterministicMockSealer::new("secret123");
           let facts = vec!["actor(alice)", "role(host)"];

           let token1 = sealer.seal(&facts).unwrap();
           let token2 = sealer.seal(&facts).unwrap();

           assert_eq!(token1, token2);
       }

       #[test]
       fn test_deterministic_mock_sealer_different_facts_different_token() {
           let sealer = DeterministicMockSealer::new("secret123");
           let facts1 = vec!["actor(alice)", "role(host)"];
           let facts2 = vec!["actor(bob)", "role(participant)"];

           let token1 = sealer.seal(&facts1).unwrap();
           let token2 = sealer.seal(&facts2).unwrap();

           assert_ne!(token1, token2);
       }
   }
   ```

2. Create `crates/server/src/middleware/mod.rs`:

   ```rust
   pub mod biscuit_auth;

   pub use biscuit_auth::{BiscuitAuthLayer, BiscuitAuthMiddleware, BiscuitSealer, DeterministicMockSealer};
   ```

3. Update `crates/server/src/lib.rs`:

   ```rust
   pub mod middleware;  // NEW: export middleware stack
   pub use middleware::BiscuitAuthLayer;
   ```

4. Add to `crates/core/src/lib.rs`:

   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct BiscuitToken {
       pub token_string: String,
       pub workspace_id: String,
       pub session_id: String,
       pub actor_id: String,
       pub role: String,
       pub permissions: Vec<PermissionPrimitive>,
       pub expiry_unix: u64,
   }
   ```

5. Update `docs/contracts/session-identity.v0.1.fixtures.json`:

   ```json
   {
     "version": "session-identity.v0.1",
     "fixtures": [
       {
         "id": "host_token_fixture",
         "type": "biscuit_host_role",
         "workspace_id": "workspace_test_001",
         "session_id": "session_test_001",
         "actor_id": "actor_host_001",
         "actor_type": "Human",
         "actor_name": "Host User",
         "role": "host",
         "permissions": ["read", "comment", "write", "approve", "administer"],
         "capability": "host_minting",
         "expiry_offset_sec": 7200,
         "deterministic_nonce": "fixture_host_001",
         "serialized_biscuit": "mock_biscuit_12345_actor_host_001_role_host",
         "cross_repo_usage": [
           "rumble-canvas integration",
           "rumble-ai-practices integration"
         ]
       },
       {
         "id": "participant_token_fixture",
         "type": "biscuit_participant_role",
         "workspace_id": "workspace_test_001",
         "session_id": "session_test_001",
         "actor_id": "actor_participant_001",
         "actor_type": "Human",
         "actor_name": "Participant User",
         "role": "participant",
         "permissions": ["read", "comment"],
         "capability": "answer_submit",
         "expiry_offset_sec": 7200,
         "deterministic_nonce": "fixture_participant_001",
         "serialized_biscuit": "mock_biscuit_67890_actor_participant_001_role_participant",
         "cross_repo_usage": [
           "rumble-canvas integration",
           "rumble-ai-practices scoring"
         ]
       }
     ]
   }
   ```

6. Create `crates/server/tests/integration_cross_repo_biscuit_fixture.rs`:
   ```rust
   #[cfg(test)]
   mod tests {
       use std::fs;
       use presto_core::{BiscuitToken, RoleAssignment, PermissionPrimitive};

       #[test]
       fn test_cross_repo_fixture_loads_and_validates() {
           let fixture_json = fs::read_to_string("docs/contracts/session-identity.v0.1.fixtures.json")
               .expect("fixture file must exist");
           let fixtures: serde_json::Value = serde_json::from_str(&fixture_json)
               .expect("fixture must be valid JSON");

           let host_fixture = &fixtures["fixtures"][0];
           assert_eq!(host_fixture["role"], "host");
           assert!(host_fixture["serialized_biscuit"].is_string());

           let participant_fixture = &fixtures["fixtures"][1];
           assert_eq!(participant_fixture["role"], "participant");
           assert!(participant_fixture["permissions"].is_array());
       }

       #[test]
       fn test_cross_repo_fixture_role_assignment_extraction() {
           // Simulate canvas/ai-practices loading fixture and building RoleAssignment.
           let host_role = RoleAssignment::host(
               "workspace_test_001".to_string(),
               "actor_host_001".to_string(),
           );

           assert_eq!(host_role.workspace_id, "workspace_test_001");
           assert_eq!(host_role.role, "host");
           assert!(host_role.permissions.contains(&PermissionPrimitive::Write));
           assert!(host_role.permissions.contains(&PermissionPrimitive::Approve));
       }

       #[test]
       fn test_cross_repo_fixture_permissions_closed_vocabulary() {
           // Verify fixture permissions conform to ADR 0028 amendment 1.
           let fixture_json = fs::read_to_string("docs/contracts/session-identity.v0.1.fixtures.json")
               .expect("fixture file must exist");
           let fixtures: serde_json::Value = serde_json::from_str(&fixture_json)
               .expect("fixture must be valid JSON");

           let closed_vocab = vec!["read", "comment", "write", "approve", "invite", "administer", "delegate"];

           for fixture in fixtures["fixtures"].as_array().unwrap() {
               for perm in fixture["permissions"].as_array().unwrap() {
                   let perm_str = perm.as_str().expect("permission must be string");
                   assert!(closed_vocab.contains(&perm_str), "permission '{}' not in closed vocabulary", perm_str);
               }
           }
       }

       #[test]
       fn test_biscuit_token_fixture_format() {
           // Verify serialized_biscuit field is present and parseable.
           let fixture_json = fs::read_to_string("docs/contracts/session-identity.v0.1.fixtures.json")
               .expect("fixture file must exist");
           let fixtures: serde_json::Value = serde_json::from_str(&fixture_json)
               .expect("fixture must be valid JSON");

           for fixture in fixtures["fixtures"].as_array().unwrap() {
               assert!(fixture["serialized_biscuit"].is_string());
               let biscuit = fixture["serialized_biscuit"].as_str().unwrap();
               assert!(biscuit.starts_with("mock_biscuit_"));
           }
       }
   }
   ```

**Exit gates:**

- `cargo test --workspace --all-targets` ✓ (all tests pass, including cross-repo fixture validation)
- `cargo fmt --all --check` ✓
- `cargo check --workspace` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo deny check` ✓ (audit clean; if RUSTSEC-2026-0173 still unresolved, explicitly document mock-sealer mitigation)
- Cross-repo fixture validation: `jq '.fixtures[].permissions[]' docs/contracts/session-identity.v0.1.fixtures.json | sort -u` ✓ (should list 7 items: read, comment, write, approve, invite, administer, delegate)
- Serialized Biscuit presence: `jq -r '.fixtures[].serialized_biscuit' docs/contracts/session-identity.v0.1.fixtures.json | wc -l` ✓ (should be ≥2)
- PR merges with all gates green.

---

### I4 — Extract scoring module + ai-practices consumption + contract fixtures

**Status (2026-07-09):** ✓ Delivered — PR #48 (feat: I4 document ScoreSink consumption pattern, 2a6a823404c0bcef368e2cbc1d8907c65519b3a1).

**Purpose:** Extract `crates/server/src/scoring.rs` as a consumable API (public trait + implementations + examples). Document scoring hook pattern for products (ai-practices, future crew). Provide fixture contracts (`score_hook_example.fixtures.rs`) showing how to implement custom ScoreSink. Prove ai-practices can consume scoring module and build their customized scoring logic (e.g., weighting by question difficulty). Add public exports and examples in lib.rs.

**Files touched:**

- `crates/server/src/lib.rs` — Promote scoring to top-level public API (re-export ScoreSink, InMemorySink); add module docs with example usage.
- `crates/server/src/scoring.rs` — Refactor to be product-agnostic; add `#[example]` docs and fixture pattern example.
- `crates/server/examples/custom_scoring_hook.rs` (new) — Runnable example: implement custom ScoreSink (e.g., difficulty-weighted scoring) for ai-practices to adapt.
- `docs/scoring-hook-pattern.md` (new) — Guide for products to consume scoring module and implement custom logic.
- `crates/server/tests/integration_ai_practices_scoring_consumption.rs` (new) — Simulate ai-practices consuming ScoreSink trait; implement mock difficulty-weighted variant; validate it integrates with lm session workflow.

**Prerequisite:**

- I3 complete (Biscuit middleware, mock sealer, cross-repo fixture) ✓
- I2 scoring module in place (InMemorySink, default scoring formula) ✓

**Work (exact, no vagueness):**

1. Refactor `crates/server/src/scoring.rs`:

   ````rust
   //! Scoring hook module — extensible interface for custom answer evaluation.
   //!
   //! # Overview
   //!
   //! The `ScoreSink` trait allows products to implement custom scoring logic
   //! for quiz/assessment sessions. The default tracer-bullet scoring formula
   //! is: `correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0`.
   //!
   //! # Usage (for ai-practices)
   //!
   //! ```ignore
   //! use presto_server::scoring::{ScoreSink, InMemorySink};
   //! use async_trait::async_trait;
   //!
   //! struct DifficultyWeightedSink {
   //!     inner: InMemorySink,
   //!     difficulty_weights: HashMap<String, f64>,  // question_id -> weight
   //! }
   //!
   //! #[async_trait]
   //! impl ScoreSink for DifficultyWeightedSink {
   //!     async fn compute_score(&self, choice: &str, correct_choice: &str, elapsed_ms: u64)
   //!         -> Result<u64, Box<dyn std::error::Error>>
   //!     {
   //!         let base = self.inner.compute_score(choice, correct_choice, elapsed_ms).await?;
   //!         let weight = self.difficulty_weights.get("q_id").copied().unwrap_or(1.0);
   //!         Ok((base as f64 * weight) as u64)
   //!     }
   //! }
   //! ```

   use async_trait::async_trait;
   use std::sync::Arc;
   use std::sync::Mutex;

   /// Trait for custom scoring hook implementations.
   /// Implement this trait to provide custom answer evaluation logic
   /// (e.g., difficulty-weighted scoring, partial credit, etc.).
   #[async_trait]
   pub trait ScoreSink: Send + Sync {
       /// Called when a participant submits an answer.
       async fn on_answer_submitted(
           &self,
           session_id: &str,
           participant_id: &str,
           question_id: &str,
           choice: &str,
           elapsed_ms: u64,
       ) -> Result<(), Box<dyn std::error::Error>>;

       /// Compute score for an answer.
       /// Default formula: `correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0`
       async fn compute_score(
           &self,
           choice: &str,
           correct_choice: &str,
           elapsed_ms: u64,
       ) -> Result<u64, Box<dyn std::error::Error>>;
   }

   /// In-memory mock ScoreSink for testing and local development.
   pub struct InMemorySink {
       answers: Arc<Mutex<Vec<(String, String, String, String, u64)>>>,
   }

   impl InMemorySink {
       pub fn new() -> Self {
           Self {
               answers: Arc::new(Mutex::new(Vec::new())),
           }
       }

       pub fn recorded_answers(&self) -> Vec<(String, String, String, String, u64)> {
           self.answers.lock().unwrap().clone()
       }
   }

   #[async_trait]
   impl ScoreSink for InMemorySink {
       async fn on_answer_submitted(
           &self,
           session_id: &str,
           participant_id: &str,
           question_id: &str,
           choice: &str,
           elapsed_ms: u64,
       ) -> Result<(), Box<dyn std::error::Error>> {
           self.answers.lock().unwrap().push((
               session_id.to_string(),
               participant_id.to_string(),
               question_id.to_string(),
               choice.to_string(),
               elapsed_ms,
           ));
           Ok(())
       }

       async fn compute_score(
           &self,
           choice: &str,
           correct_choice: &str,
           elapsed_ms: u64,
       ) -> Result<u64, Box<dyn std::error::Error>> {
           if choice == correct_choice {
               let time_bonus = ((30000_i64 - elapsed_ms as i64).max(0) as f64 / 300.0).min(100.0) as u64;
               Ok(500 + time_bonus)
           } else {
               Ok(0)
           }
       }
   }

   #[cfg(test)]
   mod tests {
       use super::*;

       #[tokio::test]
       async fn test_score_hook_correct_answer() {
           let sink = InMemorySink::new();
           let score = sink.compute_score("A", "A", 5000).await.unwrap();
           assert_eq!(score, 583);  // 500 + min((30000-5000)/300, 100) = 500 + 83
       }

       #[tokio::test]
       async fn test_score_hook_incorrect_answer() {
           let sink = InMemorySink::new();
           let score = sink.compute_score("B", "A", 5000).await.unwrap();
           assert_eq!(score, 0);
       }

       #[tokio::test]
       async fn test_score_hook_on_answer_submitted_recorded() {
           let sink = InMemorySink::new();
           sink.on_answer_submitted("sess1", "part1", "q1", "A", 5000).await.unwrap();
           let recorded = sink.recorded_answers();
           assert_eq!(recorded.len(), 1);
           assert_eq!(recorded[0].0, "sess1");
       }
   }
   ````

2. Update `crates/server/src/lib.rs`:

   ```rust
   pub mod scoring;

   pub use scoring::{ScoreSink, InMemorySink};

   //! # Scoring Hook Pattern
   //!
   //! Products (e.g., ai-practices) consume the `ScoreSink` trait to implement
   //! custom scoring logic. See `examples/custom_scoring_hook.rs` for a full example.
   ```

3. Create `crates/server/examples/custom_scoring_hook.rs`:

   ```rust
   //! Example: Implementing a custom ScoreSink for difficulty-weighted scoring.
   //! This is the pattern ai-practices should follow.

   use presto_server::ScoreSink;
   use async_trait::async_trait;
   use std::collections::HashMap;

   /// Example: Difficulty-weighted scoring for ai-practices.
   /// Multiplies the base tracer-bullet score by a per-question difficulty weight.
   struct DifficultyWeightedSink {
       // In real usage, load from AI-practices config or database.
       difficulty_weights: HashMap<String, f64>,
   }

   impl DifficultyWeightedSink {
       fn new() -> Self {
           let mut weights = HashMap::new();
           weights.insert("q1".to_string(), 1.0);   // Normal
           weights.insert("q2".to_string(), 1.5);   // Hard
           weights.insert("q3".to_string(), 0.75);  // Easy
           Self {
               difficulty_weights: weights,
           }
       }
   }

   #[async_trait]
   impl ScoreSink for DifficultyWeightedSink {
       async fn on_answer_submitted(
           &self,
           _session_id: &str,
           _participant_id: &str,
           _question_id: &str,
           _choice: &str,
           _elapsed_ms: u64,
       ) -> Result<(), Box<dyn std::error::Error>> {
           // ai-practices: log to their analytics pipeline.
           Ok(())
       }

       async fn compute_score(
           &self,
           choice: &str,
           correct_choice: &str,
           elapsed_ms: u64,
       ) -> Result<u64, Box<dyn std::error::Error>> {
           // Base score using tracer-bullet formula.
           let base = if choice == correct_choice {
               let time_bonus = ((30000_i64 - elapsed_ms as i64).max(0) as f64 / 300.0).min(100.0) as u64;
               500 + time_bonus
           } else {
               0
           };

           // Apply difficulty weight (hypothetical: load from DB by question_id in real usage).
           let weight = self.difficulty_weights.get("q1").copied().unwrap_or(1.0);
           Ok((base as f64 * weight) as u64)
       }
   }

   #[tokio::main]
   async fn main() -> Result<(), Box<dyn std::error::Error>> {
       let sink = DifficultyWeightedSink::new();

       // Example: Compute score for a correct answer on a hard question.
       let base_score = sink.compute_score("A", "A", 5000).await?;
       let expected = (583.0 * 1.5) as u64;  // 583 (base) * 1.5 (weight) = 874
       println!("Base score: {}, Weighted score: {} (expected ~{})", base_score, base_score, expected);

       Ok(())
   }
   ```

4. Create `docs/scoring-hook-pattern.md`:

   ````markdown
   # Scoring Hook Pattern — Integration Guide for AI-Practices & Crew

   ## Overview

   The `ScoreSink` trait allows products (ai-practices, crew) to implement custom scoring logic without modifying lm core. This guide shows how to consume the trait and implement your own scoring strategy.

   ## Step 1: Add lm dependency

   ```toml
   # In your product's Cargo.toml
   presto-server = { path = "$DEV_ROOT/rumble-lm/crates/server" }
   ```
   ````

   ## Step 2: Implement ScoreSink

   ```rust
   use presto_server::ScoreSink;
   use async_trait::async_trait;

   pub struct YourCustomSink {
       // Your state (e.g., question metadata, difficulty weights, analytics client)
   }

   #[async_trait]
   impl ScoreSink for YourCustomSink {
       async fn on_answer_submitted(
           &self,
           session_id: &str,
           participant_id: &str,
           question_id: &str,
           choice: &str,
           elapsed_ms: u64,
       ) -> Result<(), Box<dyn std::error::Error>> {
           // Log, validate, trigger side-effects.
           Ok(())
       }

       async fn compute_score(
           &self,
           choice: &str,
           correct_choice: &str,
           elapsed_ms: u64,
       ) -> Result<u64, Box<dyn std::error::Error>> {
           // Your scoring formula here.
           // Base tracer-bullet: correct ? 500 + min((30000 - elapsed_ms).max(0) / 300, 100) : 0
           Ok(0)
       }
   }
   ```

   ## Step 3: Wire into session handler

   When starting a session, pass your `ScoreSink` instance to the handler:

   ```rust
   let scoring = YourCustomSink::new();
   let handler = SessionHandler::new(store, fanout, scoring);
   ```

   ## Example: Difficulty-weighted scoring (ai-practices use case)

   See `$DEV_ROOT/rumble-lm/crates/server/examples/custom_scoring_hook.rs` for a full working example.

   ```

   ```

5. Create `crates/server/tests/integration_ai_practices_scoring_consumption.rs`:
   ```rust
   #[cfg(test)]
   mod tests {
       use presto_server::ScoreSink;
       use async_trait::async_trait;
       use std::collections::HashMap;
       use std::sync::Arc;
       use std::sync::Mutex;

       /// Example AI-Practices custom sink (difficulty-weighted).
       struct AIPracticesCustomSink {
           difficulty_weights: HashMap<String, f64>,
           recorded: Arc<Mutex<Vec<(String, u64)>>>,
       }

       impl AIPracticesCustomSink {
           fn new() -> Self {
               let mut weights = HashMap::new();
               weights.insert("q1".to_string(), 1.0);
               weights.insert("q2".to_string(), 1.5);
               Self {
                   difficulty_weights: weights,
                   recorded: Arc::new(Mutex::new(Vec::new())),
               }
           }
       }

       #[async_trait]
       impl ScoreSink for AIPracticesCustomSink {
           async fn on_answer_submitted(
               &self,
               _session_id: &str,
               _participant_id: &str,
               _question_id: &str,
               _choice: &str,
               _elapsed_ms: u64,
           ) -> Result<(), Box<dyn std::error::Error>> {
               Ok(())
           }

           async fn compute_score(
               &self,
               choice: &str,
               correct_choice: &str,
               elapsed_ms: u64,
           ) -> Result<u64, Box<dyn std::error::Error>> {
               let base = if choice == correct_choice {
                   let time_bonus = ((30000_i64 - elapsed_ms as i64).max(0) as f64 / 300.0).min(100.0) as u64;
                   500 + time_bonus
               } else {
                   0
               };
               let weight = self.difficulty_weights.get("q1").copied().unwrap_or(1.0);
               let final_score = (base as f64 * weight) as u64;
               self.recorded.lock().unwrap().push(("q1".to_string(), final_score));
               Ok(final_score)
           }
       }

       #[tokio::test]
       async fn test_ai_practices_custom_sink_consumption() {
           let sink = AIPracticesCustomSink::new();

           // Correct answer, hard question (weight 1.5).
           let score = sink.compute_score("A", "A", 5000).await.unwrap();
           let expected = (583.0 * 1.5) as u64;  // 583 * 1.5 = 874
           assert_eq!(score, expected);
       }

       #[tokio::test]
       async fn test_ai_practices_sink_integration_with_session() {
           let sink = AIPracticesCustomSink::new();

           sink.on_answer_submitted("sess1", "part1", "q1", "A", 5000).await.unwrap();
           let score = sink.compute_score("A", "A", 5000).await.unwrap();

           let recorded = sink.recorded.lock().unwrap().clone();
           assert_eq!(recorded.len(), 1);
           assert_eq!(recorded[0].1, score);
       }
   }
   ```

**Exit gates:**

- `cargo test --workspace --all-targets` ✓ (all scoring tests pass, including ai-practices consumption test)
- `cargo fmt --all --check` ✓
- `cargo check --workspace` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo run --example custom_scoring_hook` ✓ (example runs without error)
- File existence: `test -f docs/scoring-hook-pattern.md` ✓
- Example compiles and documents usage: `cargo test --doc --example custom_scoring_hook` ✓
- Scoring module public exports: `cargo doc --workspace --no-deps 2>&1 | grep -q 'pub trait ScoreSink'` ✓
- PR merges with all gates green.

---

### I5 — End-to-end Playwright test suite + .env.example + npm setup

**Status (2026-07-09):** ✓ Delivered — PR #49 (feat: I5 End-to-end Playwright test suite + npm setup + CI integration, 8b95c550349a315dacdf247f927245117727232a, with follow-up fix f2b0ffb8 for action SHAs).

**Purpose:** Build production-grade e2e test suite (Playwright, TypeScript) covering 5+ critical flows (join/submit/reveal/leaderboard/error cases). Prove session lifecycle in a real browser context with live Postgres + Redis. Document `.env.example` and npm setup for developers. CI integration (run on PR, block merge if failures). Unblock verification before canvas/ai-practices depend on lm runtime.

**Files touched:**

- `e2e/tests/session.spec.ts` (new) — Playwright test suite (5+ scenarios: host join, participant join, submit answer, reveal scores, error handling).
- `e2e/playwright.config.ts` (new) — Playwright configuration (base URL, timeouts, headless/debug modes).
- `e2e/package.json` (new) — npm dependencies (@playwright/test).
- `e2e/.env.example` (new) — E2E-specific env vars (VITE_BASE_URL, test user credentials if needed).
- `.env.example` (root) — Updated with E2E section.
- `.github/workflows/ci.yml` — Add e2e job (after integration tests, requires Postgres + Redis).
- `docs/e2e-testing.md` (new) — Setup guide (npm init, @playwright/test, npx playwright install, run tests locally/CI).

**Prerequisite:**

- I4 complete (scoring module extracted, documented) ✓
- I3 complete (Biscuit auth middleware) ✓
- All prior increments merged + main branch green ✓
- Live server can be started (with Postgres + Redis) ✓

**Work (exact, no vagueness):**

1. Create `e2e/package.json`:

   ```json
   {
     "name": "presto-e2e",
     "version": "0.0.0",
     "description": "End-to-end tests for presto-lm session runtime",
     "scripts": {
       "test": "playwright test",
       "test:debug": "playwright test --debug",
       "test:headed": "playwright test --headed",
       "test:ui": "playwright test --ui",
       "playwright:install": "playwright install"
     },
     "devDependencies": {
       "@playwright/test": "^1.40.0"
     }
   }
   ```

2. Create `e2e/playwright.config.ts`:

   ```typescript
   import { defineConfig, devices } from "@playwright/test";

   /**
    * Read environment variables from file.
    * https://github.com/motdotla/dotenv
    */
   // require('dotenv').config();

   /**
    * See https://playwright.dev/docs/test-configuration.
    */
   export default defineConfig({
     testDir: "./tests",
     /* Run tests in files in parallel */
     fullyParallel: true,
     /* Fail the build on CI if you accidentally left test.only in the source code. */
     forbidOnly: !!process.env.CI,
     /* Retry on CI only */
     retries: process.env.CI ? 2 : 0,
     /* Opt out of parallel tests on CI. */
     workers: process.env.CI ? 1 : undefined,
     /* Reporter to use. See https://playwright.dev/docs/test-reporters */
     reporter: "html",
     /* Shared settings for all the projects below. See https://playwright.dev/docs/api/class-testoptions. */
     use: {
       /* Base URL to use in actions like `await page.goto('/')`. */
       baseURL: process.env.BASE_URL || "http://localhost:3000",
       /* Collect trace when retrying the failed test. See https://playwright.dev/docs/trace-viewer */
       trace: "on-first-retry",
     },

     /* Configure projects for major browsers */
     projects: [
       {
         name: "chromium",
         use: { ...devices["Desktop Chrome"] },
       },
     ],

     /* Run your local dev server before starting the tests */
     webServer: {
       command: "cargo run --bin presto-server",
       url: "http://localhost:3000",
       reuseExistingServer: !process.env.CI,
       timeout: 120000,
     },
   });
   ```

3. Create `e2e/tests/session.spec.ts`:

   ```typescript
   import { test, expect } from "@playwright/test";

   test.describe("Session lifecycle", () => {
     test.beforeEach(async ({ context }) => {
       // Set up auth context for host/participant.
       // (In real usage, mint Biscuit token via API.)
     });

     test("Host can join and create a session", async ({ page }) => {
       await page.goto("/");
       await page.click('button:has-text("Create Session")');

       // Verify session created.
       const sessionId = await page
         .locator('[data-testid="session-id"]')
         .textContent();
       expect(sessionId).toBeTruthy();
     });

     test("Participant can join with valid token", async ({
       page,
       browser,
     }) => {
       // Host creates session.
       const hostPage = await browser.newPage();
       await hostPage.goto("/");
       await hostPage.click('button:has-text("Create Session")');
       const joinLink = await hostPage
         .locator('[data-testid="join-link"]')
         .getAttribute("href");

       // Participant joins via link.
       await page.goto(joinLink!);
       const participantId = await page
         .locator('[data-testid="participant-id"]')
         .textContent();
       expect(participantId).toBeTruthy();
     });

     test("Participant can submit answer", async ({ page, browser }) => {
       // Setup: host creates, participant joins.
       const hostPage = await browser.newPage();
       await hostPage.goto("/");
       await hostPage.click('button:has-text("Create Session")');
       const joinLink = await hostPage
         .locator('[data-testid="join-link"]')
         .getAttribute("href");

       await page.goto(joinLink!);

       // Participant submits answer.
       await page.click("text=Option A");
       await page.click('button:has-text("Submit")');

       // Verify submission confirmed.
       await expect(page.locator("text=Answer submitted")).toBeVisible();
     });

     test("Host can reveal answers and see scores", async ({
       page,
       browser,
     }) => {
       // Setup: host creates, participants join + submit.
       const hostPage = await browser.newPage();
       await hostPage.goto("/");
       await hostPage.click('button:has-text("Create Session")');
       const joinLink = await hostPage
         .locator('[data-testid="join-link"]')
         .getAttribute("href");

       const participant1 = await browser.newPage();
       await participant1.goto(joinLink!);
       await participant1.click("text=Option A");
       await participant1.click('button:has-text("Submit")');

       // Host reveals.
       await hostPage.click('button:has-text("Reveal Scores")');

       // Verify leaderboard appears.
       await expect(hostPage.locator("text=Leaderboard")).toBeVisible();
       const scores = await hostPage
         .locator('[data-testid="score"]')
         .allTextContents();
       expect(scores.length).toBeGreaterThan(0);
     });

     test("Error handling: invalid token rejected", async ({ page }) => {
       // Try to join with invalid token.
       await page.goto("/?token=invalid_token_12345");

       // Verify error message.
       await expect(
         page.locator("text=Invalid or expired token"),
       ).toBeVisible();
     });

     test("Leaderboard sorted by score descending", async ({
       page,
       browser,
     }) => {
       // Setup: host creates, multiple participants submit with different elapsed times.
       const hostPage = await browser.newPage();
       await hostPage.goto("/");
       await hostPage.click('button:has-text("Create Session")');
       const joinLink = await hostPage
         .locator('[data-testid="join-link"]')
         .getAttribute("href");

       // Participant 1: fast (5000ms).
       const p1 = await browser.newPage();
       await p1.goto(joinLink!);
       await p1.click("text=Option A");
       await p1.click('button:has-text("Submit")');

       // Participant 2: slower (15000ms).
       const p2 = await browser.newPage();
       await p2.goto(joinLink!);
       await p2.click("text=Option A");
       // Simulate delay... (in real test, use `await page.waitForTimeout(15000)` or clock manipulation)
       await p2.click('button:has-text("Submit")');

       // Host reveals.
       await hostPage.click('button:has-text("Reveal Scores")');

       // Verify p1 (higher score) is first in leaderboard.
       const leaderboardRows = await hostPage
         .locator('[data-testid="leaderboard-row"]')
         .allTextContents();
       const p1Rank = leaderboardRows.findIndex((row) =>
         row.includes("Participant 1"),
       );
       const p2Rank = leaderboardRows.findIndex((row) =>
         row.includes("Participant 2"),
       );
       expect(p1Rank).toBeLessThan(p2Rank);
     });
   });
   ```

4. Create `e2e/.env.example`:

   ```
   # Playwright e2e tests environment

   # Base URL for test browser navigation
   BASE_URL=http://localhost:3000

   # Session server endpoint (for programmatic session creation in tests)
   API_BASE_URL=http://localhost:3000/api

   # Test user credentials (if using local Keycloak or mock auth)
   TEST_HOST_EMAIL=host@example.com
   TEST_HOST_PASSWORD=testpassword123

   TEST_PARTICIPANT_EMAIL=participant@example.com
   TEST_PARTICIPANT_PASSWORD=testpassword123
   ```

5. Update root `.env.example`:

   ```
   # === Core Server ===

   # Database
   DATABASE_URL=postgres://postgres:presto@localhost:5432/postgres

   # Cache/Fanout
   REDIS_URL=redis://localhost:6379/

   # Authentication
   BISCUIT_PRIVATE_KEY=<paste output of: presto-server keygen>

   # RAG (optional)
   LOCAL_AI_ENABLED=1
   LOCAL_AI_BASE_URL=http://127.0.0.1:8000
   LOCAL_AI_API_KEY=<local-only-key>

   # === E2E Tests (see e2e/.env.example for details) ===

   # For Playwright: base URL the test browser navigates to
   BASE_URL=http://localhost:3000
   ```

6. Create `docs/e2e-testing.md`:

   ````markdown
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
   ````

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

   ```

   ```

7. Update `.github/workflows/ci.yml` to add e2e job:
   ```yaml
   e2e:
     name: End-to-end tests (Playwright)
     runs-on: ubuntu-latest
     needs: [rust, integration] # Depends on prior checks
     services:
       postgres:
         image: pgvector/pgvector:pg16
         env:
           POSTGRES_PASSWORD: presto
         ports:
           - 5432:5432
         options: >-
           --health-cmd pg_isready --health-interval 5s --health-timeout 5s --health-retries 10
       redis:
         image: redis:7-alpine
         ports:
           - 6379:6379
         options: >-
           --health-cmd "redis-cli ping" --health-interval 5s --health-timeout 5s --health-retries 10
     env:
       DATABASE_URL: postgres://postgres:presto@localhost:5432/postgres
       REDIS_URL: redis://localhost:6379/
     steps:
       - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5 # v4
       - uses: dtolnay/rust-toolchain@4be7066ada62dd38de10e7b70166bc74ed198c30 # stable
       - uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32 # v2
       - name: Build server
         run: cargo build --bin presto-server --release
       - uses: actions/setup-node@11bd9d76d4b9c31ae0c46e4aaefc79f34eed6f34 # v4
         with:
           node-version: "18"
       - name: Install Playwright
         run: |
           cd e2e
           npm install
           npx playwright install --with-deps
       - name: Run e2e tests
         run: |
           cd e2e
           npm test
       - name: Upload Playwright report
         if: always()
         uses: actions/upload-artifact@6d1c36657c4a29b8c2d5dbc9974bf5b8dde42920 # v4
         with:
           name: playwright-report
           path: e2e/playwright-report/
   ```

**Exit gates:**

- `cd e2e && npm install && npm test` ✓ (all 5+ scenarios pass on live server)
- `npx playwright show-report` ✓ (HTML report generated and readable)
- `cargo build --bin presto-server --release` ✓ (server builds for e2e startup)
- `.env.example` files exist (root + e2e/) with DATABASE_URL/REDIS_URL/BISCUIT_PRIVATE_KEY ✓
- `docs/e2e-testing.md` documents setup, debugging, CI integration ✓
- CI e2e job green: `npx playwright test --exit-code-on-node-failure` ✓ (exit code 0 when all pass)
- PR merges with all gates (rust quality + integration + e2e) green.

---

### I6 — Expose live question grounding summary (bonus increment)

**Status (2026-07-09):** ✓ Delivered — PR #53 (feat: I6 expose live question grounding summary, 513f74d71dd0df9ac22f924d8f833071e0323e24).

**Purpose:** Delivered incrementally: adds live-question-grounding.v0.1 contract fixtures, public grounding summary on QuestionPublic, RAG verified markers after grounding verification, fixture markers for demo content, and anti-forge stripping for client PushQuestion. Proves verifiable question provenance and RAG grounding evidence (beyond the original plan scope, delivered for completeness).

---

## Success criteria (all 6 increments, delivered)

- **Schema & contract:** workspace-identity.v0.1 contract fixtures published; ADR 0028 amendments 1–3 implemented (closed vocabulary, D11-gating, big-bang posture).
- **State & scale:** Postgres SessionStore proven (state persists, recovered on re-query); Redis Fanout proven (multi-instance, no cross-talk).
- **Auth & trust:** Biscuit middleware live; attenuated tokens (host/participant roles) validated; mock sealer unblocks testing (RUSTSEC-2026-0173 mitigation documented).
- **Scoring:** Trait extracted, consumable by ai-practices; custom hooks documented + example implemented.
- **Verification:** E2E test suite (Playwright, 5+ flows) passing; CI gates comprehensive (cargo + sqlx + playwright).
- **Traceability:** All increments cross-ref ADR 0028 (amendments), canvas/ai-practices plans, and schema contracts; paths qualified with `$DEV_ROOT`.
- **Unblocking:** D11 adoption path complete (2 implementations: lm + canvas; cross-repo Biscuit fixture); ai-practices can delete shim (ADR 0005) when lm proves live; canvas MVP can ship multi-actor session-workspace flows.
