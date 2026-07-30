import { beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { withTenantDbTransaction } from "@libre-ai/data";
import { createTestDatabase, type TestDatabase } from "@libre-ai/testing";

// The RGPD tables of 0002_rgpd.sql exercised against the real PostgreSQL
// barrier (PGlite): FORCE RLS tenant isolation, append-only grants
// (SELECT/INSERT — tombstones and audit rows are evidence, like
// deletion_receipts), and the structural CHECKs on tenant and digest formats.
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
const DIGEST = "d".repeat(64);
const NOW = "2026-07-23T10:00:00Z";

let tdb: TestDatabase;

beforeAll(async () => {
  tdb = await createTestDatabase();
  await tdb.applyMigrations(DATA_MIGRATIONS);
  await tdb.applyMigrations(SESSIONS_MIGRATIONS);
});

describe("session_deleted_subjects", () => {
  test("tombstones are tenant-isolated under the app role", async () => {
    await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      await tx.query(
        `INSERT INTO session_deleted_subjects (tenant_id, subject_digest, receipt_id, deleted_at)
         VALUES ($1, $2, $3, $4)`,
        [TENANT_A, DIGEST, "rcpt_isolation", NOW],
      );
    });
    const foreign = await withTenantDbTransaction(tdb.db, TENANT_B, (tx) =>
      tx.query("SELECT subject_digest FROM session_deleted_subjects"),
    );
    expect(foreign.rows).toHaveLength(0);
    const own = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query("SELECT subject_digest FROM session_deleted_subjects"),
    );
    expect(own.rows).toHaveLength(1);
  });

  test("is append-only for the app role: UPDATE and DELETE are rejected", async () => {
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
        tx.query("UPDATE session_deleted_subjects SET receipt_id = 'rewritten'"),
      ),
    ).rejects.toThrow();
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
        tx.query("DELETE FROM session_deleted_subjects"),
      ),
    ).rejects.toThrow();
  });

  test("rejects malformed tenant and digest at the structural CHECK", async () => {
    await expect(
      tdb.db.query(
        `INSERT INTO session_deleted_subjects (tenant_id, subject_digest, receipt_id, deleted_at)
         VALUES ('not-a-tenant', $1, 'rcpt_bad', $2)`,
        [DIGEST, NOW],
      ),
    ).rejects.toThrow();
    await expect(
      tdb.db.query(
        `INSERT INTO session_deleted_subjects (tenant_id, subject_digest, receipt_id, deleted_at)
         VALUES ($1, 'user@example.com', 'rcpt_bad', $2)`,
        [TENANT_A, NOW],
      ),
    ).rejects.toThrow();
  });
});

describe("session_events.actor_digest (0003)", () => {
  test("a human event without its digest is structurally rejected", async () => {
    await expect(
      tdb.db.query(
        `INSERT INTO session_events
           (tenant_id, session_id, sequence, event_id, revision, type, actor_kind, actor_id, actor_digest, occurred_at, data, recorded_at)
         VALUES ($1, 'urn:libre-ai:session:s-floor', 1, 'urn:libre-ai:event:e-floor', 0,
                 'session-created', 'human', 'owner-alpha', NULL, $2, '{}', $2)`,
        [TENANT_A, NOW],
      ),
    ).rejects.toThrow();
  });

  test("a malformed digest is rejected, a system actor carries none", async () => {
    await expect(
      tdb.db.query(
        `INSERT INTO session_events
           (tenant_id, session_id, sequence, event_id, revision, type, actor_kind, actor_id, actor_digest, occurred_at, data, recorded_at)
         VALUES ($1, 'urn:libre-ai:session:s-floor', 1, 'urn:libre-ai:event:e-floor', 0,
                 'session-created', 'human', 'owner-alpha', 'user@example.com', $2, '{}', $2)`,
        [TENANT_A, NOW],
      ),
    ).rejects.toThrow();
    await tdb.db.query(
      `INSERT INTO session_events
         (tenant_id, session_id, sequence, event_id, revision, type, actor_kind, actor_id, actor_digest, occurred_at, data, recorded_at)
       VALUES ($1, 'urn:libre-ai:session:s-floor-sys', 1, 'urn:libre-ai:event:e-floor-sys', 0,
               'session-created', 'system', 'scheduler', NULL, $2, '{}', $2)`,
      [TENANT_A, NOW],
    );
  });
});

describe("0003 backfill on a non-empty log", () => {
  test("pre-existing human rows get the exact deriveSubjectDigest value", async () => {
    // A deployment whose log already holds human events must be able to
    // apply 0003: the in-SQL backfill must reproduce byte-for-byte the
    // TypeScript derivation the RGPD read paths use.
    const { deriveSubjectDigest } = await import("@libre-ai/rgpd-kit");
    const { readFile, readdir } = await import("node:fs/promises");
    const staged = await createTestDatabase();
    try {
      await staged.applyMigrations(DATA_MIGRATIONS);
      const files = (await readdir(SESSIONS_MIGRATIONS)).filter((f) => f.endsWith(".sql")).sort();
      for (const file of files.filter((f) => !f.startsWith("0003"))) {
        await staged.db.exec(await readFile(join(SESSIONS_MIGRATIONS, file), "utf8"));
      }
      await staged.db.query(
        `INSERT INTO session_events
           (tenant_id, session_id, sequence, event_id, revision, type, actor_kind, actor_id, occurred_at, data, recorded_at)
         VALUES ($1, 'urn:libre-ai:session:s-legacy', 1, 'urn:libre-ai:event:e-legacy', 0,
                 'session-created', 'human', 'owner-legacy', $2, '{}', $2)`,
        [TENANT_A, NOW],
      );
      const migration0003 = files.find((f) => f.startsWith("0003"));
      if (migration0003 === undefined) throw new Error("0003 migration missing");
      await staged.db.exec(await readFile(join(SESSIONS_MIGRATIONS, migration0003), "utf8"));
      const backfilled = await staged.db.query<{ actor_digest: string }>(
        "SELECT actor_digest FROM session_events WHERE actor_id = 'owner-legacy'",
      );
      expect(backfilled.rows[0]?.actor_digest).toBe(
        await deriveSubjectDigest(TENANT_A, "owner-legacy"),
      );
    } finally {
      await staged.close();
    }
  });
});

describe("session_subject_audit", () => {
  test("audit rows are tenant-isolated and append-only", async () => {
    await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
      await tx.query(
        `INSERT INTO session_subject_audit
           (tenant_id, request_id, subject_digest, right_type, status, detail, recorded_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)`,
        [TENANT_A, "dsr_audit_1", DIGEST, "erasure", "received", null, NOW],
      );
    });
    const foreign = await withTenantDbTransaction(tdb.db, TENANT_B, (tx) =>
      tx.query("SELECT request_id FROM session_subject_audit"),
    );
    expect(foreign.rows).toHaveLength(0);
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
        tx.query("UPDATE session_subject_audit SET status = 'fulfilled'"),
      ),
    ).rejects.toThrow();
    await expect(
      withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
        tx.query("DELETE FROM session_subject_audit"),
      ),
    ).rejects.toThrow();
  });

  test("constrains right_type and status to the locked vocabularies", async () => {
    await expect(
      tdb.db.query(
        `INSERT INTO session_subject_audit
           (tenant_id, request_id, subject_digest, right_type, status, detail, recorded_at)
         VALUES ($1, 'dsr_audit_2', $2, 'deletion', 'received', NULL, $3)`,
        [TENANT_A, DIGEST, NOW],
      ),
    ).rejects.toThrow();
    await expect(
      tdb.db.query(
        `INSERT INTO session_subject_audit
           (tenant_id, request_id, subject_digest, right_type, status, detail, recorded_at)
         VALUES ($1, 'dsr_audit_3', $2, 'erasure', 'done', NULL, $3)`,
        [TENANT_A, DIGEST, NOW],
      ),
    ).rejects.toThrow();
  });
});
