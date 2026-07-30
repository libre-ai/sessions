import { beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { type SqlExecutor, withTenantDbTransaction } from "@libre-ai/data";
import { createTestDatabase, type TestDatabase } from "@libre-ai/testing";
import {
  confirmLift,
  isRestricted,
  RestrictionStateError,
  requestLift,
  restrict,
} from "./restriction";

// The restriction state store (Art. 18) exercised against the real barrier
// (PGlite): the append-only current-state-by-max-entry_seq model, the two-step
// lift state machine and its synthetic lift_<entry_seq> ids, the typed state
// error, and the append-only grants + tenant isolation of
// session_restricted_subjects. Every write goes through withTenantDbTransaction
// so RLS and the least-privilege grant actually bite.
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
const NOW = "2026-07-23T10:00:00Z";

let tdb: TestDatabase;

function inTenant<T>(tenantId: string, fn: (tx: SqlExecutor) => Promise<T>): Promise<T> {
  return withTenantDbTransaction(tdb.db, tenantId, fn);
}

function stateOf(tenantId: string, digest: string): Promise<boolean> {
  return inTenant(tenantId, (tx) => isRestricted(tx, tenantId, digest));
}

beforeAll(async () => {
  tdb = await createTestDatabase();
  await tdb.applyMigrations(DATA_MIGRATIONS);
  await tdb.applyMigrations(SESSIONS_MIGRATIONS);
});

describe("isRestricted truth table (highest entry_seq wins)", () => {
  test("no rows for the subject → false", async () => {
    expect(await stateOf(TENANT_A, "1".padStart(64, "a"))).toBe(false);
  });

  test("restricted → true, lift-pending → true, lifted → false", async () => {
    const digest = "b".repeat(64);
    await inTenant(TENANT_A, (tx) =>
      restrict(tx, TENANT_A, digest, "accuracy-contested", "dsr_1", NOW),
    );
    expect(await stateOf(TENANT_A, digest)).toBe(true);

    await inTenant(TENANT_A, (tx) => requestLift(tx, TENANT_A, digest, NOW));
    expect(await stateOf(TENANT_A, digest)).toBe(true);

    await inTenant(TENANT_A, (tx) => confirmLift(tx, TENANT_A, digest, NOW));
    expect(await stateOf(TENANT_A, digest)).toBe(false);
  });

  test("a later restricted row after a lift wins over the earlier lifted row", async () => {
    const digest = "c".repeat(64);
    await inTenant(TENANT_A, (tx) =>
      restrict(tx, TENANT_A, digest, "accuracy-contested", "dsr_2", NOW),
    );
    await inTenant(TENANT_A, (tx) => requestLift(tx, TENANT_A, digest, NOW));
    await inTenant(TENANT_A, (tx) => confirmLift(tx, TENANT_A, digest, NOW));
    expect(await stateOf(TENANT_A, digest)).toBe(false);
    // Re-restriction is a NEW row with a higher entry_seq — it wins.
    await inTenant(TENANT_A, (tx) =>
      restrict(tx, TENANT_A, digest, "objection-pending", "dsr_3", NOW),
    );
    expect(await stateOf(TENANT_A, digest)).toBe(true);
  });
});

describe("two-step lift under the fixed injected clock", () => {
  test("requestLift then confirmLift: audit pair joined by the synthetic lift id", async () => {
    const digest = "d".repeat(64);
    await inTenant(TENANT_A, (tx) =>
      restrict(tx, TENANT_A, digest, "needed-for-legal-claims", "dsr_4", NOW),
    );
    const restrictedSeq = await inTenant(TENANT_A, async (tx) => {
      const row = await tx.query<{ entry_seq: string | number }>(
        "SELECT entry_seq FROM session_restricted_subjects WHERE subject_digest = $1 AND state = 'restricted'",
        [digest],
      );
      return row.rows[0]?.entry_seq;
    });
    const liftId = `lift_${restrictedSeq}`;

    // Step 1: notice pending.
    await inTenant(TENANT_A, (tx) => requestLift(tx, TENANT_A, digest, NOW));
    expect(await stateOf(TENANT_A, digest)).toBe(true);
    const pending = await inTenant(TENANT_A, (tx) =>
      tx.query<{ request_id: string; recorded_at: string }>(
        "SELECT request_id, recorded_at FROM session_restricted_subjects WHERE subject_digest = $1 AND state = 'lift-pending'",
        [digest],
      ),
    );
    expect(pending.rows[0]?.request_id).toBe(liftId);
    const auditPending = await inTenant(TENANT_A, (tx) =>
      tx.query<{ status: string; detail: string; request_id: string; right_type: string }>(
        "SELECT status, detail, request_id, right_type FROM session_subject_audit WHERE request_id = $1",
        [liftId],
      ),
    );
    expect(auditPending.rows).toEqual([
      {
        status: "in-progress",
        detail: "sessions.rgpd.notice_required",
        request_id: liftId,
        right_type: "restriction",
      },
    ]);

    // Step 2: notice attested.
    await inTenant(TENANT_A, (tx) => confirmLift(tx, TENANT_A, digest, NOW));
    expect(await stateOf(TENANT_A, digest)).toBe(false);
    const auditFull = await inTenant(TENANT_A, (tx) =>
      tx.query<{ status: string; detail: string; request_id: string }>(
        "SELECT status, detail, request_id FROM session_subject_audit WHERE request_id = $1 ORDER BY status",
        [liftId],
      ),
    );
    // Same synthetic id joins the in-progress notice to the fulfilled attestation.
    expect(auditFull.rows).toEqual([
      { status: "fulfilled", detail: "sessions.rgpd.notice_attested", request_id: liftId },
      { status: "in-progress", detail: "sessions.rgpd.notice_required", request_id: liftId },
    ]);
  });
});

describe("state-machine strictness (typed error, code only)", () => {
  test("requestLift on a non-restricted subject throws RestrictionStateError", async () => {
    const absent = "e".repeat(64);
    await expect(
      inTenant(TENANT_A, (tx) => requestLift(tx, TENANT_A, absent, NOW)),
    ).rejects.toThrow(RestrictionStateError);
    // A lifted subject is not restricted either.
    const lifted = "f".repeat(64);
    await inTenant(TENANT_A, (tx) =>
      restrict(tx, TENANT_A, lifted, "accuracy-contested", "dsr_5", NOW),
    );
    await inTenant(TENANT_A, (tx) => requestLift(tx, TENANT_A, lifted, NOW));
    await inTenant(TENANT_A, (tx) => confirmLift(tx, TENANT_A, lifted, NOW));
    await expect(
      inTenant(TENANT_A, (tx) => requestLift(tx, TENANT_A, lifted, NOW)),
    ).rejects.toThrow(RestrictionStateError);
  });

  test("confirmLift without a pending lift throws RestrictionStateError with a code", async () => {
    const digest = "0".repeat(64);
    await inTenant(TENANT_A, (tx) =>
      restrict(tx, TENANT_A, digest, "accuracy-contested", "dsr_6", NOW),
    );
    let caught: unknown;
    try {
      await inTenant(TENANT_A, (tx) => confirmLift(tx, TENANT_A, digest, NOW));
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(RestrictionStateError);
    expect((caught as RestrictionStateError).code).toBe("sessions.rgpd.restriction_state_invalid");
    // The message carries the code only — no identifiers, no digest.
    expect((caught as RestrictionStateError).message).not.toContain(digest);
  });
});

describe("tenant isolation and append-only floor", () => {
  test("restriction state never leaks across the app barrier", async () => {
    const digest = "9".repeat(64);
    await inTenant(TENANT_A, (tx) =>
      restrict(tx, TENANT_A, digest, "accuracy-contested", "dsr_7", NOW),
    );
    expect(await stateOf(TENANT_A, digest)).toBe(true);
    // The same digest in tenant B is unknown — RLS scopes the read.
    expect(await stateOf(TENANT_B, digest)).toBe(false);
    const foreign = await inTenant(TENANT_B, (tx) =>
      tx.query("SELECT subject_digest FROM session_restricted_subjects"),
    );
    expect(foreign.rows).toHaveLength(0);
  });

  test("UPDATE and DELETE on session_restricted_subjects are rejected for the app role", async () => {
    await expect(
      inTenant(TENANT_A, (tx) =>
        tx.query("UPDATE session_restricted_subjects SET state = 'lifted'"),
      ),
    ).rejects.toThrow(/permission denied/);
    await expect(
      inTenant(TENANT_A, (tx) => tx.query("DELETE FROM session_restricted_subjects")),
    ).rejects.toThrow(/permission denied/);
  });

  test("the ground-iff-restricted CHECK rejects a restricted row without a ground", async () => {
    await expect(
      tdb.db.query(
        `INSERT INTO session_restricted_subjects
           (tenant_id, subject_digest, state, ground, request_id, recorded_at)
         VALUES ($1, $2, 'restricted', NULL, 'dsr_bad', $3)`,
        [TENANT_A, "8".repeat(64), NOW],
      ),
    ).rejects.toThrow();
    await expect(
      tdb.db.query(
        `INSERT INTO session_restricted_subjects
           (tenant_id, subject_digest, state, ground, request_id, recorded_at)
         VALUES ($1, $2, 'lifted', 'accuracy-contested', 'dsr_bad', $3)`,
        [TENANT_A, "7".repeat(64), NOW],
      ),
    ).rejects.toThrow();
  });
});
