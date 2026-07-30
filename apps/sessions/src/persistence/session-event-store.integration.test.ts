import { beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { withTenantDbTransaction } from "@libre-ai/data";
import { createTestDatabase, type TestDatabase } from "@libre-ai/testing";
import { type SessionEvent, validateEvent } from "../domain/session-event";
import {
  appendEvent,
  loadEvents,
  loadSessionState,
  SessionSequenceConflictError,
  SessionStreamCorruptError,
  SessionTenantMismatchError,
} from "./session-event-store";

// The session-event persistence exercised against the real PostgreSQL barrier
// (PGlite): the append-only session_events table, its FORCE RLS policy and its
// SELECT/INSERT-only grant from 0001_sessions.sql, on top of the libre_ai_app
// role from packages/data.
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
const NOW = "2030-01-01T00:00:00Z";

let tdb: TestDatabase;

beforeAll(async () => {
  tdb = await createTestDatabase();
  await tdb.applyMigrations(DATA_MIGRATIONS);
  await tdb.applyMigrations(SESSIONS_MIGRATIONS);
});

function event(overrides: Record<string, unknown>): SessionEvent {
  const outcome = validateEvent({
    schemaVersion: "libre-ai.session-event.v1",
    id: "urn:libre-ai:event:e-1",
    tenantId: TENANT_A,
    sessionId: "urn:libre-ai:session:s-default",
    sequence: 1,
    revision: 0,
    type: "session-created",
    actor: { kind: "human", id: "owner-alpha" },
    occurredAt: "2030-01-01T00:00:00Z",
    data: {},
    ...overrides,
  });
  if (!outcome.ok) throw new Error(`fixture invalid: ${outcome.refusal}`);
  return outcome.value;
}

async function asRawTenant<T>(tenant: string, fn: () => Promise<T>): Promise<T> {
  await tdb.db.exec("BEGIN");
  try {
    await tdb.db.exec("SET LOCAL ROLE libre_ai_app");
    await tdb.db.query("SELECT set_config('app.tenant_id', $1, true)", [tenant]);
    return await fn();
  } finally {
    await tdb.db.exec("ROLLBACK");
  }
}

describe("session event store — round-trip and reduction", () => {
  test("appends a stream and reloads it in causal order", async () => {
    const session = "urn:libre-ai:session:rt1";
    const created = event({ sessionId: session });
    const joined = event({
      sessionId: session,
      id: "urn:libre-ai:event:e-2",
      sequence: 2,
      revision: 1,
      type: "participant-joined",
    });

    const events = await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      await appendEvent(tx, created, NOW);
      await appendEvent(tx, joined, NOW);
      return loadEvents(tx, session);
    });
    expect(events.map((e) => e.sequence)).toEqual([1, 2]);
    expect(events[1]?.type).toBe("participant-joined");
  });

  test("rebuilds state by folding the persisted stream through the reducer", async () => {
    const session = "urn:libre-ai:session:rt2";
    const created = event({ sessionId: session });
    const closed = event({
      sessionId: session,
      id: "urn:libre-ai:event:e-2",
      sequence: 2,
      revision: 1,
      type: "session-closed",
    });

    const state = await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      await appendEvent(tx, created, NOW);
      await appendEvent(tx, closed, NOW);
      return loadSessionState(tx, session);
    });
    expect(state?.eventCount).toBe(2);
    expect(state?.latestSequence).toBe(2);
    expect(state?.closed).toBe(true);
  });

  test("an unknown session reduces to null", async () => {
    const state = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      loadSessionState(tx, "urn:libre-ai:session:none"),
    );
    expect(state).toBeNull();
  });

  test("a persisted stream that does not reduce surfaces SessionStreamCorruptError", async () => {
    // appendEvent enforces no sequence contiguity (that is the reducer's job), so
    // a seq-1 then seq-3 stream persists but cannot be folded back into a state.
    const session = "urn:libre-ai:session:corrupt1";
    const created = event({ sessionId: session });
    const gap = event({
      sessionId: session,
      id: "urn:libre-ai:event:e-3",
      sequence: 3,
      revision: 1,
      type: "participant-joined",
    });
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
        await appendEvent(tx, created, NOW);
        await appendEvent(tx, gap, NOW);
        return loadSessionState(tx, session);
      }),
    ).rejects.toBeInstanceOf(SessionStreamCorruptError);
  });
});

describe("session event store — append-only and concurrency", () => {
  test("a replayed sequence conflicts rather than forking", async () => {
    const session = "urn:libre-ai:session:cc1";
    const created = event({ sessionId: session });
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
        await appendEvent(tx, created, NOW);
        await appendEvent(tx, created, NOW);
      }),
    ).rejects.toBeInstanceOf(SessionSequenceConflictError);
  });

  test("the app role cannot UPDATE the log (append-only grant)", async () => {
    await asRawTenant(TENANT_A, async () => {
      await expect(tdb.db.query("UPDATE session_events SET revision = 99")).rejects.toThrow();
    });
  });

  test("the app role cannot DELETE from the log (append-only grant)", async () => {
    await asRawTenant(TENANT_A, async () => {
      await expect(tdb.db.query("DELETE FROM session_events")).rejects.toThrow();
    });
  });
});

describe("session event store — tenant isolation", () => {
  test("another tenant cannot read a session's stream (RLS)", async () => {
    const session = "urn:libre-ai:session:iso1";
    await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      appendEvent(tx, event({ sessionId: session }), NOW),
    );
    const crossTenant = await withTenantDbTransaction(tdb.db, TENANT_B, (tx) =>
      loadEvents(tx, session),
    );
    expect(crossTenant).toEqual([]);
  });

  test("appending an event whose tenant differs from the context is rejected", async () => {
    const foreign = event({ sessionId: "urn:libre-ai:session:mm1", tenantId: TENANT_B });
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) => appendEvent(tx, foreign, NOW)),
    ).rejects.toBeInstanceOf(SessionTenantMismatchError);
  });
});
