import { beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { withTenantDbTransaction } from "@libre-ai/data";
import { createTestDatabase, type TestDatabase } from "@libre-ai/testing";
import { loadEvents } from "../persistence/session-event-store";
import { executeSessionCommand, type SessionPrincipal } from "./execute-command";

// The sessions command service exercised end-to-end against the real PostgreSQL
// barrier (PGlite): validate -> authorize -> reduce -> append, in one tenant
// transaction, on top of 0001_sessions.sql and the packages/data app role.
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
const OWNER: SessionPrincipal = { role: "owner" };
const PARTICIPANT: SessionPrincipal = { role: "participant" };

let tdb: TestDatabase;

beforeAll(async () => {
  tdb = await createTestDatabase();
  await tdb.applyMigrations(DATA_MIGRATIONS);
  await tdb.applyMigrations(SESSIONS_MIGRATIONS);
});

function raw(overrides: Record<string, unknown>): Record<string, unknown> {
  return {
    schemaVersion: "libre-ai.session-event.v1",
    id: "urn:libre-ai:event:e-1",
    tenantId: TENANT_A,
    sessionId: "urn:libre-ai:session:s-default",
    sequence: 1,
    revision: 0,
    type: "session-created",
    actor: { kind: "human", id: "owner-alpha" },
    occurredAt: NOW,
    data: {},
    ...overrides,
  };
}

describe("executeSessionCommand — accepted vertical", () => {
  test("an owner opens a session then adds a member", async () => {
    const session = "urn:libre-ai:session:cs1";
    const result = await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      const created = await executeSessionCommand(tx, OWNER, raw({ sessionId: session }), NOW);
      expect(created.status).toBe("accepted");
      return executeSessionCommand(
        tx,
        OWNER,
        raw({
          sessionId: session,
          id: "urn:libre-ai:event:e-2",
          sequence: 2,
          revision: 1,
          type: "member-added",
        }),
        NOW,
      );
    });
    expect(result.status).toBe("accepted");
    if (result.status !== "accepted") return;
    expect(result.state.eventCount).toBe(2);
    expect(result.state.latestSequence).toBe(2);
  });
});

describe("executeSessionCommand — refused, fail-closed", () => {
  test("a participant may not create a session, and nothing is written", async () => {
    const session = "urn:libre-ai:session:cs-unauth";
    const [result, events] = await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      const r = await executeSessionCommand(tx, PARTICIPANT, raw({ sessionId: session }), NOW);
      return [r, await loadEvents(tx, session)] as const;
    });
    expect(result).toEqual({ status: "refused", refusal: "sessions.membership_required" });
    expect(events).toEqual([]);
  });

  test("a structurally malformed event is cursor_invalid", async () => {
    const result = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      executeSessionCommand(tx, OWNER, raw({ tenantId: "org-example" }), NOW),
    );
    expect(result).toEqual({ status: "refused", refusal: "sessions.cursor_invalid" });
  });

  test("a first event that is not session-created is cursor_invalid", async () => {
    const result = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      executeSessionCommand(
        tx,
        OWNER,
        raw({ sessionId: "urn:libre-ai:session:cs-first", type: "member-added" }),
        NOW,
      ),
    );
    expect(result).toEqual({ status: "refused", refusal: "sessions.cursor_invalid" });
  });

  test("a sequence gap is cursor_invalid", async () => {
    const session = "urn:libre-ai:session:cs-gap";
    const result = await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      await executeSessionCommand(tx, OWNER, raw({ sessionId: session }), NOW);
      return executeSessionCommand(
        tx,
        OWNER,
        raw({
          sessionId: session,
          id: "urn:libre-ai:event:e-3",
          sequence: 3,
          revision: 1,
          type: "member-added",
        }),
        NOW,
      );
    });
    expect(result).toEqual({ status: "refused", refusal: "sessions.cursor_invalid" });
  });

  test("an event from another tenant is tenant_mismatch", async () => {
    const session = "urn:libre-ai:session:cs-tenant";
    const result = await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      await executeSessionCommand(tx, OWNER, raw({ sessionId: session }), NOW);
      return executeSessionCommand(
        tx,
        OWNER,
        raw({
          sessionId: session,
          tenantId: TENANT_B,
          id: "urn:libre-ai:event:e-2",
          sequence: 2,
          revision: 1,
          type: "member-added",
        }),
        NOW,
      );
    });
    expect(result).toEqual({ status: "refused", refusal: "sessions.tenant_mismatch" });
  });

  test("a foreign-tenant first event is tenant_mismatch (stopped at the append barrier)", async () => {
    // reduce(null, ...) has no prior state to compare tenants against, so the
    // append barrier is the sole check here — it must surface a clean refusal,
    // not throw.
    const result = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      executeSessionCommand(
        tx,
        OWNER,
        raw({ sessionId: "urn:libre-ai:session:cs-tenant-first", tenantId: TENANT_B }),
        NOW,
      ),
    );
    expect(result).toEqual({ status: "refused", refusal: "sessions.tenant_mismatch" });
  });

  test("a rewound revision is revision_stale", async () => {
    const session = "urn:libre-ai:session:cs-rev";
    const result = await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      await executeSessionCommand(tx, OWNER, raw({ sessionId: session, revision: 2 }), NOW);
      return executeSessionCommand(
        tx,
        OWNER,
        raw({
          sessionId: session,
          id: "urn:libre-ai:event:e-2",
          sequence: 2,
          revision: 1,
          type: "member-added",
        }),
        NOW,
      );
    });
    expect(result).toEqual({ status: "refused", refusal: "sessions.revision_stale" });
  });
});
