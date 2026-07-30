import { afterAll, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import {
  BelowMinimumRetentionError,
  getDeletionReceipt,
  InMemoryBlobStore,
  InMemoryProjectionCache,
  runRetentionSweep,
  upsertRetentionRule,
  withTenantDbTransaction,
  withTenantRetentionTransaction,
} from "@libre-ai/data";
import { deriveSubjectDigest } from "@libre-ai/rgpd-kit";
import { createTestDatabase, type TestDatabase } from "@libre-ai/testing";
import {
  type ActorKind,
  type EventType,
  type SessionEvent,
  validateEvent,
} from "../domain/session-event";
import { appendEvent, loadSessionState } from "../persistence/session-event-store";
import { createSessionsDataSubjectRights } from "./data-subject-rights";
import { confirmLift, requestLift } from "./restriction";
import { sessionsCompactionSpec } from "./retention";

// The FULL acceptance suite for the Sessions retention compaction spec
// (retention design §6, complete, plus restriction design §6.7 both
// directions) exercised end to end against the real barrier (PGlite):
// the dedicated retention role and its owner-declared grants (0005), the
// G-A session-lifecycle sweep, the in-transaction phase-2 re-check (TOCTOU
// guards, both arms), the restricted/lift-pending exclusion, and the
// erasure-compaction evidence (compactedReceiptIds). `recorded_at` is the
// age column (server-set at append; occurred_at is client-supplied).

const DATA_MIGRATIONS = join(
  import.meta.dir,
  "..",
  "..",
  "..",
  "..",
  "packages",
  "data",
  "migrations",
);
const SESSIONS_MIGRATIONS = join(import.meta.dir, "..", "..", "migrations");

const TENANT_A = "ten_aaaaaaaaaaaaaaaa";
const TENANT_B = "ten_bbbbbbbbbbbbbbbb";

// Sweep clock and the two ends of the P90D window (cutoff = NOW - 90d =
// 2026-04-25). OLD events are past the window (fully expired); FRESH events
// are inside it; MID is old enough to fall past a tightened P7D window but
// not the default P90D (§6.6).
const NOW = "2026-07-24T00:00:00Z";
const OLD = "2026-01-01T00:00:00Z";
const FRESH = "2026-07-20T00:00:00Z";
const MID = "2026-07-01T00:00:00Z";

// A real contract rule (contracts/data/retention.v1.json): default P90D,
// configurable P7D..P365D — the store bound-checks configured windows.
const SESSIONS_CONTENT_RULE = {
  id: "sessions-content",
  mode: "fixed",
  defaultRetention: "P90D",
  configurable: { minimum: "P7D", maximum: "P365D" },
};

const CONTRIBUTION_DATA = {
  resourceId: "urn:libre-ai:resource:r-1",
  audience: "session",
  contentDigest: "c".repeat(64),
} as const;

let tdb: TestDatabase;
let eventSeq = 0;

interface SeedEvent {
  readonly sequence: number;
  readonly type: EventType;
  readonly actorId: string;
  readonly recordedAt: string;
  readonly actorKind?: ActorKind;
  readonly data?: Record<string, unknown>;
}

function buildEvent(tenantId: string, sessionId: string, spec: SeedEvent): SessionEvent {
  eventSeq += 1;
  const outcome = validateEvent({
    schemaVersion: "libre-ai.session-event.v1",
    id: `urn:libre-ai:event:ev-${eventSeq}`,
    tenantId,
    sessionId,
    sequence: spec.sequence,
    // Revision tracks the sequence so it never rewinds (reducer invariant).
    revision: spec.sequence - 1,
    type: spec.type,
    actor: { kind: spec.actorKind ?? "human", id: spec.actorId },
    // occurred_at is irrelevant to the sweep (age = recorded_at); reuse the
    // injected recorded_at so the fixture is a valid RFC-3339 timestamp.
    occurredAt: spec.recordedAt,
    data: spec.data ?? {},
  });
  if (!outcome.ok) throw new Error(`fixture invalid: ${outcome.refusal}`);
  return outcome.value;
}

/** Append a whole session stream under the app barrier; recorded_at per event. */
async function seedSession(
  tenantId: string,
  sessionId: string,
  events: readonly SeedEvent[],
): Promise<void> {
  await withTenantDbTransaction(tdb.db, tenantId, async (tx) => {
    for (const spec of events) {
      await appendEvent(tx, buildEvent(tenantId, sessionId, spec), spec.recordedAt);
    }
  });
}

/** A standard three-event stream authored by owner + one member. */
function memberStream(memberId: string, recordedAt: string): readonly SeedEvent[] {
  return [
    { sequence: 1, type: "session-created", actorId: "owner-alpha", recordedAt },
    { sequence: 2, type: "participant-joined", actorId: memberId, recordedAt },
    {
      sequence: 3,
      type: "contribution-submitted",
      actorId: memberId,
      recordedAt,
      data: CONTRIBUTION_DATA,
    },
  ];
}

function digest(tenantId: string, actorId: string): Promise<string> {
  return deriveSubjectDigest(tenantId, actorId);
}

// Superuser count (bypasses RLS) — the assertion oracle, like D1's probeCount.
async function eventCount(tenantId: string, sessionId: string): Promise<number> {
  const res = await tdb.db.query<{ n: number }>(
    "SELECT count(*)::int AS n FROM session_events WHERE tenant_id = $1 AND session_id = $2",
    [tenantId, sessionId],
  );
  return res.rows[0]?.n ?? -1;
}

function makePort() {
  let counter = 0;
  return createSessionsDataSubjectRights({
    executor: tdb.db,
    cache: new InMemoryProjectionCache(),
    blobs: new InMemoryBlobStore(),
    now: () => NOW,
    newRequestId: () => {
      counter += 1;
      return `dsr_it_${counter}`;
    },
  });
}

function sweep(tenantId: string, now: string = NOW) {
  return runRetentionSweep(tdb.db, sessionsCompactionSpec, tenantId, now);
}

beforeAll(async () => {
  tdb = await createTestDatabase();
  await tdb.applyMigrations(DATA_MIGRATIONS);
  await tdb.applyMigrations(SESSIONS_MIGRATIONS);
});

afterAll(async () => {
  await tdb.close();
});

beforeEach(async () => {
  // Each test controls its own fixtures: reset every evidence and content
  // table under the superuser (RLS-bypassing) so counts are hermetic.
  await tdb.db.exec(`
    DELETE FROM session_events;
    DELETE FROM session_restricted_subjects;
    DELETE FROM session_deleted_subjects;
    DELETE FROM session_subject_audit;
    DELETE FROM deletion_receipts;
    DELETE FROM retention_rules;
  `);
});

describe("§6.1 — retention role probe and app-floor regression", () => {
  test("libre_ai_retention is NOLOGIN, NOSUPERUSER, NOBYPASSRLS", async () => {
    const res = await tdb.db.query<{
      rolcanlogin: boolean;
      rolsuper: boolean;
      rolbypassrls: boolean;
    }>(
      "SELECT rolcanlogin, rolsuper, rolbypassrls FROM pg_roles WHERE rolname = 'libre_ai_retention'",
    );
    expect(res.rows[0]).toEqual({ rolcanlogin: false, rolsuper: false, rolbypassrls: false });
  });

  test("app floor untouched: UPDATE/DELETE on session_events rejected under the app barrier", async () => {
    await seedSession(TENANT_A, "urn:libre-ai:session:s-floor", memberStream("member-alice", OLD));
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
        tx.query("DELETE FROM session_events WHERE session_id = 'urn:libre-ai:session:s-floor'"),
      ),
    ).rejects.toThrow(/permission denied/);
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
        tx.query(
          "UPDATE session_events SET revision = 99 WHERE session_id = 'urn:libre-ai:session:s-floor'",
        ),
      ),
    ).rejects.toThrow(/permission denied/);
    // The app-role attempt changed nothing.
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-floor")).toBe(3);
  });
});

describe("§6.2 — FORCE RLS binds the retention role", () => {
  test("a retention-barrier delete under tenant B spares tenant A entirely", async () => {
    // Both tenants hold a fully-expired session. The retention role has no
    // BYPASSRLS, and the FORCE RLS policy carries no TO clause, so it binds
    // the retention role too: a delete scoped to B's GUC cannot touch A.
    await seedSession(TENANT_A, "urn:libre-ai:session:s-a", memberStream("member-alice", OLD));
    await seedSession(TENANT_B, "urn:libre-ai:session:s-b", memberStream("member-bob", OLD));

    await withTenantRetentionTransaction(tdb.db, TENANT_B, async (tx) => {
      // No WHERE clause: RLS — not an explicit predicate — is what spares A.
      await tx.query("DELETE FROM session_events");
    });

    expect(await eventCount(TENANT_B, "urn:libre-ai:session:s-b")).toBe(0);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-a")).toBe(3);
  });

  test("a full sweep for tenant B deletes zero rows of tenant A", async () => {
    await seedSession(TENANT_A, "urn:libre-ai:session:s-a", memberStream("member-alice", OLD));
    const report = await sweep(TENANT_B);
    expect(report.tenantId).toBe(TENANT_B);
    expect(report.sessionsSelected).toBe(0);
    expect(report.sessionsDeleted).toBe(0);
    // A's expired data is untouched by B's sweep.
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-a")).toBe(3);
  });
});

describe("§6.3 — G-A window sweep", () => {
  test("a fully-expired session disappears atomically; a session with one fresh event stays whole", async () => {
    await seedSession(
      TENANT_A,
      "urn:libre-ai:session:s-expired",
      memberStream("member-alice", OLD),
    );
    // s-fresh: two OLD events but ONE fresh contribution — max(recorded_at) is
    // fresh, so the WHOLE session is retained (no row-level holes).
    await seedSession(TENANT_A, "urn:libre-ai:session:s-fresh", [
      { sequence: 1, type: "session-created", actorId: "owner-alpha", recordedAt: OLD },
      { sequence: 2, type: "participant-joined", actorId: "member-bob", recordedAt: OLD },
      {
        sequence: 3,
        type: "contribution-submitted",
        actorId: "member-bob",
        recordedAt: FRESH,
        data: CONTRIBUTION_DATA,
      },
    ]);

    const report = await sweep(TENANT_A);
    expect(report.sessionsSelected).toBe(1);
    expect(report.sessionsDeleted).toBe(1);
    expect(report.eventsDeleted).toBe(3);
    expect(report.compactedReceiptIds).toEqual([]);

    // Only the expired session's rows are gone; the fresh session is whole.
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-expired")).toBe(0);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-fresh")).toBe(3);

    // Every remaining session still folds through loadSessionState.
    const state = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      loadSessionState(tx, "urn:libre-ai:session:s-fresh"),
    );
    expect(state).not.toBeNull();
    expect(state?.eventCount).toBe(3);
    // The compacted session is simply absent (null), never corrupt.
    const gone = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      loadSessionState(tx, "urn:libre-ai:session:s-expired"),
    );
    expect(gone).toBeNull();
  });
});

describe("§7.3 — recorded_at is the sole age column", () => {
  test("a FRESH occurred_at cannot keep an OLD-recorded_at session alive: the sweep deletes it", async () => {
    // Every other fixture in this file ties occurred_at to the same injected
    // recordedAt (buildEvent), so a regression that swapped the sweep's age
    // column to the client-forgeable occurred_at would pass the whole suite.
    // The log is append-only (no UPDATE can retrofit a discriminating row
    // onto a normal seedSession() stream), so seed the raw row directly, the
    // way the migration test does (migration.integration.test.ts) via the
    // superuser oracle, matching 0001/0003's columns exactly. occurred_at and
    // recorded_at sit at OPPOSITE ends of the window: occurred_at = FRESH
    // (inside it) but recorded_at = OLD (past it). Sequence 1 keeps the
    // stream causally valid (session-created, the only legal opener).
    const sessionId = "urn:libre-ai:session:s-age-column";
    await tdb.db.query(
      `INSERT INTO session_events
         (tenant_id, session_id, sequence, event_id, revision, type,
          actor_kind, actor_id, actor_digest, occurred_at, data, recorded_at)
       VALUES ($1, $2, 1, 'urn:libre-ai:event:e-age-column', 0,
               'session-created', 'system', 'scheduler', NULL, $3, '{}', $4)`,
      [TENANT_A, sessionId, FRESH, OLD],
    );

    const report = await sweep(TENANT_A);
    expect(report.sessionsSelected).toBe(1);
    expect(report.sessionsDeleted).toBe(1);
    expect(report.eventsDeleted).toBe(1);

    // A regression measuring age on occurred_at would have kept this session
    // alive (occurred_at is FRESH); recorded_at (server-set) is OLD, so the
    // sweep must physically delete it.
    expect(await eventCount(TENANT_A, sessionId)).toBe(0);
  });
});

describe("§6.4 — erasure compaction (deferred-compaction evidenced)", () => {
  test("once the containing session expires it is swept; tombstone/audit/receipt REMAIN and the report names the receipt", async () => {
    const port = makePort();
    const aliceDigest = await digest(TENANT_A, "member-alice");
    await seedSession(TENANT_A, "urn:libre-ai:session:s-erase", memberStream("member-alice", OLD));

    // Art. 17: a real tombstone + receipt via the port.
    const erased = await port.handleErasureRequest(TENANT_A, aliceDigest);
    expect(erased.status).toBe("fulfilled");
    if (erased.status !== "fulfilled") return;
    const receiptId = erased.deletionReceiptId;

    // The request-handler flow records the erasure in the audit trail; the port
    // path under test does not, so seed the fulfilled audit row (app-role
    // INSERT) that a full request would have written — to prove the SWEEP never
    // deletes it (audit rows are evidence, never swept).
    await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query(
        `INSERT INTO session_subject_audit
           (tenant_id, request_id, subject_digest, right_type, status, detail, receipt_id, recorded_at)
         VALUES ($1, $2, $3, 'erasure', 'fulfilled', NULL, $4, $5)`,
        [TENANT_A, erased.requestId, aliceDigest, receiptId, NOW],
      ),
    );

    const report = await sweep(TENANT_A);
    expect(report.sessionsDeleted).toBe(1);
    expect(report.eventsDeleted).toBe(3);
    // The erased subject's receipt is the deferred-compaction cross-check.
    expect(report.compactedReceiptIds).toEqual([receiptId]);

    // Alice's rows are physically gone.
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-erase")).toBe(0);

    // The evidence survives the compaction: tombstone, receipt, audit rows.
    const tombstones = await tdb.db.query<{ subject_digest: string }>(
      "SELECT subject_digest FROM session_deleted_subjects WHERE tenant_id = $1",
      [TENANT_A],
    );
    expect(tombstones.rows).toEqual([{ subject_digest: aliceDigest }]);
    const receipt = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      getDeletionReceipt(tx, receiptId),
    );
    expect(receipt?.id).toBe(receiptId);
    const audit = await tdb.db.query<{ n: number }>(
      "SELECT count(*)::int AS n FROM session_subject_audit WHERE tenant_id = $1",
      [TENANT_A],
    );
    expect(audit.rows[0]?.n).toBeGreaterThan(0);

    // Access still refuses the erased subject.
    expect(await port.handleAccessRequest(TENANT_A, aliceDigest)).toMatchObject({
      status: "refused",
      refusal: "sessions.rgpd.subject_erased",
    });
  });
});

describe("§6.5 / restriction §6.7 — restriction exclusion, both directions", () => {
  test("a restricted then lift-pending subject's expired session survives; after confirmLift a new sweep deletes it", async () => {
    const port = makePort();
    const carolDigest = await digest(TENANT_A, "member-carol");
    await seedSession(
      TENANT_A,
      "urn:libre-ai:session:s-restricted",
      memberStream("member-carol", OLD),
    );

    // Art. 18(1): restrict the subject. The whole session must be excluded.
    expect(
      (await port.handleRestrictionRequest(TENANT_A, carolDigest, "needed-for-legal-claims"))
        .status,
    ).toBe("fulfilled");

    const first = await sweep(TENANT_A);
    expect(first.sessionsSelected).toBe(0); // phase-1 exclusion
    expect(first.sessionsDeleted).toBe(0);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-restricted")).toBe(3);

    // Art. 18(3) step 1: lift-pending is still a restriction — still excluded.
    await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      requestLift(tx, TENANT_A, carolDigest, NOW),
    );
    const second = await sweep(TENANT_A);
    expect(second.sessionsSelected).toBe(0);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-restricted")).toBe(3);

    // Art. 18(3) step 2: the lift completes — the session is now compactable.
    await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      confirmLift(tx, TENANT_A, carolDigest, NOW),
    );
    const third = await sweep(TENANT_A);
    expect(third.sessionsSelected).toBe(1);
    expect(third.sessionsDeleted).toBe(1);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-restricted")).toBe(0);
  });

  test("a restricted-then-erased subject is compactable once expired (erasure supersedes the restriction)", async () => {
    const port = makePort();
    const daveDigest = await digest(TENANT_A, "member-dave");
    await seedSession(TENANT_A, "urn:libre-ai:session:s-re", memberStream("member-dave", OLD));

    // Restrict, then erase at the subject's OWN request (Art. 18(2) consent to
    // that one processing): the erasure supersedes and the session compacts.
    expect(
      (await port.handleRestrictionRequest(TENANT_A, daveDigest, "objection-pending")).status,
    ).toBe("fulfilled");
    const erased = await port.handleErasureRequest(TENANT_A, daveDigest);
    expect(erased.status).toBe("fulfilled");
    if (erased.status !== "fulfilled") return;

    const report = await sweep(TENANT_A);
    expect(report.sessionsSelected).toBe(1);
    expect(report.sessionsDeleted).toBe(1);
    expect(report.compactedReceiptIds).toEqual([erased.deletionReceiptId]);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-re")).toBe(0);
  });
});

describe("§6.6 — tenant-configurable window", () => {
  test("a shorter valid window makes an already-old session eligible; an out-of-bounds value is refused", async () => {
    await seedSession(TENANT_A, "urn:libre-ai:session:s-mid", memberStream("member-alice", MID));

    // Under the default P90D window the MID session is not yet expired.
    const before = await sweep(TENANT_A);
    expect(before.sessionsSelected).toBe(0);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-mid")).toBe(3);

    // Tighten to P7D (valid, >= minimum): the session becomes eligible.
    await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      upsertRetentionRule(tx, {
        rule: SESSIONS_CONTENT_RULE,
        requested: "P7D",
        updatedBy: "usr_owner",
        updatedAt: NOW,
      }),
    );
    const after = await sweep(TENANT_A);
    expect(after.sessionsSelected).toBe(1);
    expect(after.sessionsDeleted).toBe(1);
    expect(await eventCount(TENANT_A, "urn:libre-ai:session:s-mid")).toBe(0);

    // Below the minimum (P1D < P7D): refused by the store validation.
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
        upsertRetentionRule(tx, {
          rule: SESSIONS_CONTENT_RULE,
          requested: "P1D",
          updatedBy: "usr_owner",
          updatedAt: NOW,
        }),
      ),
    ).rejects.toThrow(BelowMinimumRetentionError);
  });
});

describe("§6.7 — report opacity", () => {
  test("the report carries only aggregate counts, opaque receipt ids and rule/owner/tenant/timestamp — no subject data", async () => {
    const port = makePort();
    const frankDigest = await digest(TENANT_A, "member-frank");
    await seedSession(TENANT_A, "urn:libre-ai:session:s-op", memberStream("member-frank", OLD));
    const erased = await port.handleErasureRequest(TENANT_A, frankDigest);
    expect(erased.status).toBe("fulfilled");

    const report = await sweep(TENANT_A);

    expect(Object.keys(report).sort()).toEqual(
      [
        "compactedReceiptIds",
        "eventsDeleted",
        "owner",
        "ruleId",
        "sessionsDeleted",
        "sessionsSelected",
        "sweptAt",
        "tenantId",
      ].sort(),
    );
    expect(report.owner).toBe("sessions");
    expect(report.tenantId).toBe(TENANT_A);
    expect(report.ruleId).toBe("sessions-content");
    expect(report.sweptAt).toBe(NOW);

    // No subject digest or actor id from the fixtures leaks into the report.
    const serialized = JSON.stringify(report);
    expect(serialized).not.toContain(frankDigest);
    expect(serialized).not.toContain("member-frank");
    expect(serialized).not.toContain("owner-alpha");
  });
});

describe("§6.8 — TOCTOU phase-2 re-check (both arms)", () => {
  test("arm A — a fresh event appended after selection makes compactUnit return deleted:false, stream intact", async () => {
    const sessionId = "urn:libre-ai:session:s-toctou-a";
    await seedSession(TENANT_A, sessionId, memberStream("member-alice", OLD));
    // The race: a fresh event lands between the advisory selection and the
    // bounded compaction transaction.
    await seedSession(TENANT_A, sessionId, [
      { sequence: 4, type: "session-closed", actorId: "owner-alpha", recordedAt: FRESH },
    ]);

    const outcome = await withTenantRetentionTransaction(tdb.db, TENANT_A, (tx) =>
      sessionsCompactionSpec.compactUnit(tx, TENANT_A, sessionId, NOW, 90),
    );
    expect(outcome).toEqual({ deleted: false, eventsDeleted: 0, compactedReceiptIds: [] });
    expect(await eventCount(TENANT_A, sessionId)).toBe(4);
    const state = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      loadSessionState(tx, sessionId),
    );
    expect(state?.eventCount).toBe(4);
  });

  test("arm B — a restriction landing after selection makes compactUnit return deleted:false, stream intact", async () => {
    const port = makePort();
    const sessionId = "urn:libre-ai:session:s-toctou-b";
    const erinDigest = await digest(TENANT_A, "member-erin");
    await seedSession(TENANT_A, sessionId, memberStream("member-erin", OLD));
    // The race: the subject is restricted between selection and compaction.
    expect(
      (await port.handleRestrictionRequest(TENANT_A, erinDigest, "accuracy-contested")).status,
    ).toBe("fulfilled");

    const outcome = await withTenantRetentionTransaction(tdb.db, TENANT_A, (tx) =>
      sessionsCompactionSpec.compactUnit(tx, TENANT_A, sessionId, NOW, 90),
    );
    expect(outcome).toEqual({ deleted: false, eventsDeleted: 0, compactedReceiptIds: [] });
    expect(await eventCount(TENANT_A, sessionId)).toBe(3);
  });
});
