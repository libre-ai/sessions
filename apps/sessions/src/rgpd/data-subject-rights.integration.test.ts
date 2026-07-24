import { beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import {
  getDeletionReceipt,
  InMemoryBlobStore,
  InMemoryProjectionCache,
  withTenantDbTransaction,
} from "@libre-ai/data";
import { deriveSubjectDigest } from "@libre-ai/rgpd-kit";
import { createTestDatabase, type TestDatabase } from "@libre-ai/testing";
import { type SessionEvent, validateEvent } from "../domain/session-event";
import { appendEvent } from "../persistence/session-event-store";
import { createSessionsDataSubjectRights } from "./data-subject-rights";
import { confirmLift, requestLift } from "./restriction";

// The Sessions adoption of DataSubjectRightsPort exercised end to end against
// the real barrier (PGlite): RLS tenant scoping, append-only grants, the
// tombstone written inside the accepted deletion transaction and the
// deletion receipt persisted by @libre-ai/data.
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
let requestCounter = 0;

function fixture(overrides: Record<string, unknown>): SessionEvent {
  const outcome = validateEvent({
    schemaVersion: "libre-ai.session-event.v1",
    id: "urn:libre-ai:event:e-1",
    tenantId: TENANT_A,
    sessionId: "urn:libre-ai:session:s-rgpd",
    sequence: 1,
    revision: 0,
    type: "session-created",
    actor: { kind: "human", id: "owner-alpha" },
    occurredAt: NOW,
    data: {},
    ...overrides,
  });
  if (!outcome.ok) throw new Error(`fixture invalid: ${outcome.refusal}`);
  return outcome.value;
}

function makePort() {
  return createSessionsDataSubjectRights({
    executor: tdb.db,
    cache: new InMemoryProjectionCache(),
    blobs: new InMemoryBlobStore(),
    now: () => NOW,
    newRequestId: () => {
      requestCounter += 1;
      return `dsr_it_${requestCounter}`;
    },
  });
}

beforeAll(async () => {
  tdb = await createTestDatabase();
  await tdb.applyMigrations(DATA_MIGRATIONS);
  await tdb.applyMigrations(SESSIONS_MIGRATIONS);

  // Tenant A: one session created by owner-alpha, member-alice joins and
  // contributes. Tenant B: the same "member-alice" identifier in a different
  // tenant (digests must not correlate across contexts).
  const sessionA = "urn:libre-ai:session:s-rgpd-a";
  await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
    await appendEvent(tx, fixture({ sessionId: sessionA }), NOW);
    await appendEvent(
      tx,
      fixture({
        sessionId: sessionA,
        id: "urn:libre-ai:event:e-2",
        sequence: 2,
        revision: 1,
        type: "participant-joined",
        actor: { kind: "human", id: "member-alice" },
      }),
      NOW,
    );
    await appendEvent(
      tx,
      fixture({
        sessionId: sessionA,
        id: "urn:libre-ai:event:e-3",
        sequence: 3,
        revision: 2,
        type: "contribution-submitted",
        actor: { kind: "human", id: "member-alice" },
        data: {
          resourceId: "urn:libre-ai:resource:r-1",
          audience: "session",
          contentDigest: "c".repeat(64),
        },
      }),
      NOW,
    );
  });
  const sessionB = "urn:libre-ai:session:s-rgpd-b";
  await withTenantDbTransaction(tdb.db, TENANT_B, async (tx) => {
    await appendEvent(
      tx,
      fixture({
        sessionId: sessionB,
        tenantId: TENANT_B,
        actor: { kind: "human", id: "owner-beta" },
      }),
      NOW,
    );
    await appendEvent(
      tx,
      fixture({
        sessionId: sessionB,
        tenantId: TENANT_B,
        id: "urn:libre-ai:event:e-2",
        sequence: 2,
        revision: 1,
        type: "participant-joined",
        actor: { kind: "human", id: "member-alice" },
      }),
      NOW,
    );
  });
});

describe("verifySubject", () => {
  test("returns the tenant-scoped digest for a participant of the tenant", async () => {
    const port = makePort();
    const digest = await port.verifySubject(TENANT_A, "member-alice");
    expect(digest).toBe(await deriveSubjectDigest(TENANT_A, "member-alice"));
  });

  test("returns null for an unknown identifier and never leaks other tenants", async () => {
    const port = makePort();
    expect(await port.verifySubject(TENANT_A, "member-ghost")).toBeNull();
    // owner-beta only exists in tenant B; tenant A must not see them.
    expect(await port.verifySubject(TENANT_A, "owner-beta")).toBeNull();
  });

  test("returns null for an invalid identifier instead of throwing", async () => {
    const port = makePort();
    expect(await port.verifySubject(TENANT_A, "   ")).toBeNull();
  });
});

describe("handleAccessRequest", () => {
  test("exports only the subject's rows in the subject's tenant", async () => {
    const port = makePort();
    const digest = await deriveSubjectDigest(TENANT_A, "member-alice");
    const result = await port.handleAccessRequest(TENANT_A, digest);
    expect(result.status).toBe("fulfilled");
    if (result.status === "fulfilled") {
      const dataExport = result.dataExport as {
        schemaVersion: string;
        events: readonly { type: string; sequence: number }[];
      };
      expect(dataExport.schemaVersion).toBe("libre-ai.sessions.subject-export.v1");
      expect(dataExport.events.map((event) => event.type)).toEqual([
        "participant-joined",
        "contribution-submitted",
      ]);
      expect(result.categories).toEqual(["communication", "audit", "timestamp"]);
    }
  });

  test("refuses an unknown subject with a typed code", async () => {
    const port = makePort();
    const result = await port.handleAccessRequest(TENANT_A, "e".repeat(64));
    expect(result).toMatchObject({ status: "refused", refusal: "sessions.rgpd.subject_unknown" });
  });
});

describe("handleErasureRequest", () => {
  test("erases inside one accepted transaction: tombstone + receipt + counts", async () => {
    const port = makePort();
    const digest = await deriveSubjectDigest(TENANT_A, "member-alice");
    const result = await port.handleErasureRequest(TENANT_A, digest);
    expect(result.status).toBe("fulfilled");
    if (result.status !== "fulfilled") return;
    expect(result.recordsAffected).toBe(2);
    expect(result.erasedAt).toBe(NOW);
    expect(result.categoriesErased).toEqual(["communication", "audit", "timestamp"]);

    const receipt = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      getDeletionReceipt(tx, result.deletionReceiptId),
    );
    expect(receipt?.subjectDigests).toEqual([digest]);
    expect(receipt?.owner).toBe("sessions");
    expect(receipt?.status).toBe("complete");
    // Contract conformance (deletion-receipt.v1 requestedBy pattern):
    // self-service attribution as usr_ + digest prefix, no plaintext.
    expect(receipt?.requestedBy).toBe(`usr_${digest.slice(0, 32)}`);
    // Append-only log: the receipt qualifies the accepted logical deletion
    // so an auditor sees physical compaction is deferred to retention.
    const postgresql = receipt?.stores.find((store) => store.store === "postgresql");
    expect(postgresql?.outcome).toBe("deleted");
    expect(postgresql?.reasonCode).toBe("deletion.deferred-compaction");

    const tombstones = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query<{ subject_digest: string; receipt_id: string }>(
        "SELECT subject_digest, receipt_id FROM session_deleted_subjects",
      ),
    );
    expect(tombstones.rows).toEqual([
      { subject_digest: digest, receipt_id: result.deletionReceiptId },
    ]);
  });

  test("access after erasure and double erasure refuse with typed codes", async () => {
    const port = makePort();
    const digest = await deriveSubjectDigest(TENANT_A, "member-alice");
    expect(await port.handleAccessRequest(TENANT_A, digest)).toMatchObject({
      status: "refused",
      refusal: "sessions.rgpd.subject_erased",
    });
    expect(await port.handleErasureRequest(TENANT_A, digest)).toMatchObject({
      status: "refused",
      refusal: "sessions.rgpd.already_erased",
    });
  });

  test("erasure in one tenant leaves the same identifier untouched elsewhere", async () => {
    const port = makePort();
    const digestB = await deriveSubjectDigest(TENANT_B, "member-alice");
    const result = await port.handleAccessRequest(TENANT_B, digestB);
    expect(result.status).toBe("fulfilled");
  });

  test("refuses an unknown subject without writing anything", async () => {
    const port = makePort();
    const result = await port.handleErasureRequest(TENANT_A, "f".repeat(64));
    expect(result).toMatchObject({ status: "refused", refusal: "sessions.rgpd.subject_unknown" });
    const receipts = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query("SELECT receipt_id FROM deletion_receipts"),
    );
    // Only the receipt of the successful erasure above exists.
    expect(receipts.rows).toHaveLength(1);
  });
});

describe("portability, deferred rights and categories", () => {
  test("portability (Art. 20) exports the subject's rows in a machine-readable format", async () => {
    const port = makePort();
    const digest = await deriveSubjectDigest(TENANT_B, "member-alice");
    const result = await port.handlePortabilityRequest(TENANT_B, digest);
    expect(result.status).toBe("fulfilled");
    if (result.status === "fulfilled") {
      expect(result.format).toBe("application/json");
      const dataExport = result.dataExport as {
        schemaVersion: string;
        events: readonly { type: string }[];
      };
      expect(dataExport.schemaVersion).toBe("libre-ai.sessions.subject-export.v1");
      expect(dataExport.events.map((event) => event.type)).toEqual(["participant-joined"]);
    }
  });

  test("portability refuses an erased subject like access does", async () => {
    const port = makePort();
    const digest = await deriveSubjectDigest(TENANT_A, "member-alice");
    expect(await port.handlePortabilityRequest(TENANT_A, digest)).toMatchObject({
      status: "refused",
      refusal: "sessions.rgpd.subject_erased",
    });
  });

  test("declares communication/audit/timestamp on the sessions-content rule", async () => {
    const port = makePort();
    const declarations = await port.listDataCategories(TENANT_B, "a".repeat(64));
    expect(declarations.map((declaration) => declaration.category)).toEqual([
      "communication",
      "audit",
      "timestamp",
    ]);
    for (const declaration of declarations) {
      expect(declaration.retentionRule).toBe("sessions-content");
      expect(declaration.erasureScope).toBe("deferred");
      expect(declaration.legalBasis).toBe("contract");
    }
  });
});

// Art. 18 restriction at the port surface: the ground-carrying fulfillment,
// the refusal ladder (erased → unknown → already-restricted), and the proof
// that pausing processing does NOT block the subject's own Art. 15/17/20
// rights (restriction protects the subject, it does not lock them out).
describe("handleRestrictionRequest (Art. 18)", () => {
  test("restricts a known subject: fulfilled carries the ground, state row records it", async () => {
    const port = makePort();
    const digest = await deriveSubjectDigest(TENANT_A, "owner-alpha");
    const result = await port.handleRestrictionRequest(TENANT_A, digest, "accuracy-contested");
    expect(result.status).toBe("fulfilled");
    if (result.status !== "fulfilled") return;
    // The ground belongs to the subject and rides back on the fulfillment.
    expect(result.ground).toBe("accuracy-contested");
    expect(result.restrictedAt).toBe(NOW);
    // owner-alpha authored exactly the session-created event — one paused record.
    expect(result.affectedRecords).toBe(1);

    const state = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query<{ state: string; ground: string; request_id: string }>(
        "SELECT state, ground, request_id FROM session_restricted_subjects WHERE subject_digest = $1 ORDER BY entry_seq DESC LIMIT 1",
        [digest],
      ),
    );
    expect(state.rows[0]).toEqual({
      state: "restricted",
      ground: "accuracy-contested",
      request_id: result.requestId,
    });
  });

  test("refusal ladder: already-restricted, unknown subject, erased subject", async () => {
    const port = makePort();
    const ownerAlpha = await deriveSubjectDigest(TENANT_A, "owner-alpha");
    // owner-alpha was restricted by the previous test — a second request refuses.
    expect(
      await port.handleRestrictionRequest(TENANT_A, ownerAlpha, "objection-pending"),
    ).toMatchObject({ status: "refused", refusal: "sessions.rgpd.already_restricted" });
    // A digest that never authored an event is unknown.
    expect(
      await port.handleRestrictionRequest(TENANT_A, "e".repeat(64), "accuracy-contested"),
    ).toMatchObject({ status: "refused", refusal: "sessions.rgpd.subject_unknown" });
    // member-alice was erased (tombstoned) by the erasure suite above.
    const erased = await deriveSubjectDigest(TENANT_A, "member-alice");
    expect(
      await port.handleRestrictionRequest(TENANT_A, erased, "accuracy-contested"),
    ).toMatchObject({
      status: "refused",
      refusal: "sessions.rgpd.subject_erased",
    });
  });

  test("after confirmLift, the PORT restricts the same subject again: fulfilled", async () => {
    const port = makePort();
    const ownerAlpha = await deriveSubjectDigest(TENANT_A, "owner-alpha");
    // owner-alpha has been `restricted` since the first test in this describe
    // block (confirmed `already_restricted` by the refusal-ladder test above).
    // Lift through the two-step machine (restriction.ts, not the port — the
    // port has no lift verb), then prove the PORT's own refusal ladder — not
    // just isRestricted() — now treats the subject as restrictable again.
    await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      requestLift(tx, TENANT_A, ownerAlpha, NOW),
    );
    await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      confirmLift(tx, TENANT_A, ownerAlpha, NOW),
    );
    const result = await port.handleRestrictionRequest(TENANT_A, ownerAlpha, "accuracy-contested");
    expect(result.status).toBe("fulfilled");
  });

  // Ordering dependency: the last test below erases TENANT_B's member-alice
  // through the port (handleErasureRequest), and erasure is irreversible. The
  // "portability (Art. 20) exports the subject's rows" test above reads this
  // same actor while it is still live. This MUST stay the LAST test in the
  // file — any new, non-destructive test belongs ABOVE it, never appended
  // after.
  test("a restricted subject still exercises access, portability and erasure", async () => {
    const port = makePort();
    const digest = await deriveSubjectDigest(TENANT_B, "member-alice");
    expect(
      (await port.handleRestrictionRequest(TENANT_B, digest, "needed-for-legal-claims")).status,
    ).toBe("fulfilled");
    // Art. 15/20/17 serve the subject regardless of the restriction flag.
    expect((await port.handleAccessRequest(TENANT_B, digest)).status).toBe("fulfilled");
    expect((await port.handlePortabilityRequest(TENANT_B, digest)).status).toBe("fulfilled");
    expect((await port.handleErasureRequest(TENANT_B, digest)).status).toBe("fulfilled");
  });
});
