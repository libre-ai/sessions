import { beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import {
  InMemoryBlobStore,
  InMemoryProjectionCache,
  withTenantDbTransaction,
} from "@libre-ai/data";
import { createTestDatabase, type TestDatabase } from "@libre-ai/testing";
import { type SessionEvent, validateEvent } from "../domain/session-event";
import { appendEvent } from "../persistence/session-event-store";
import { createSessionsDataSubjectRights } from "./data-subject-rights";
import { createDataSubjectRequestHandler } from "./request-handler";

// The data-subject request handler exercised end to end on PGlite. The
// handler is an exported factory, deliberately NOT mounted on the public
// cockpit routes: the Sessions runtime boundary stays locked until
// WP-G3-S01's sessions-authz-review human gate.
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
const NOW = "2026-07-23T10:00:00Z";
const SESSION = "urn:libre-ai:session:s-handler";

let tdb: TestDatabase;
let requestCounter = 0;

function fixture(overrides: Record<string, unknown>): SessionEvent {
  const outcome = validateEvent({
    schemaVersion: "libre-ai.session-event.v1",
    id: "urn:libre-ai:event:e-1",
    tenantId: TENANT_A,
    sessionId: SESSION,
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

function makeHandler(
  role: "owner" | "facilitator" | "participant" | "observer",
  principalTenantId: string = TENANT_A,
) {
  const port = createSessionsDataSubjectRights({
    executor: tdb.db,
    cache: new InMemoryProjectionCache(),
    blobs: new InMemoryBlobStore(),
    now: () => NOW,
    newRequestId: () => {
      requestCounter += 1;
      return `dsr_port_${requestCounter}`;
    },
  });
  return createDataSubjectRequestHandler({
    port,
    executor: tdb.db,
    principal: { role, tenantId: principalTenantId },
    now: () => NOW,
    newRequestId: () => {
      requestCounter += 1;
      return `dsr_api_${requestCounter}`;
    },
  });
}

function post(body: unknown): Request {
  return new Request("https://sessions.test/api/data-subject-request", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

async function auditRows(requestId: string) {
  return withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
    tx.query<{ status: string; detail: string | null; receipt_id: string | null }>(
      "SELECT status, detail, receipt_id FROM session_subject_audit WHERE request_id = $1 ORDER BY status",
      [requestId],
    ),
  );
}

beforeAll(async () => {
  tdb = await createTestDatabase();
  await tdb.applyMigrations(DATA_MIGRATIONS);
  await tdb.applyMigrations(SESSIONS_MIGRATIONS);
  await withTenantDbTransaction(tdb.db, TENANT_A, async (tx) => {
    await appendEvent(tx, fixture({}), NOW);
    await appendEvent(
      tx,
      fixture({
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

describe("transport-level failures", () => {
  test("GET is 405 and an unparsable body is 400, nothing recorded", async () => {
    const handler = makeHandler("owner");
    const get = await handler(new Request("https://sessions.test/api/data-subject-request"));
    expect(get.status).toBe(405);
    const bad = await handler(
      new Request("https://sessions.test/api/data-subject-request", {
        method: "POST",
        body: "not json",
      }),
    );
    expect(bad.status).toBe(400);
    const malformed = await handler(post({ rightType: "erasure" }));
    expect(malformed.status).toBe(400);
  });
});

describe("authorization is deny-by-default before any I/O", () => {
  test("an observer requesting erasure is 403 with no audit trace", async () => {
    const handler = makeHandler("observer");
    const response = await handler(
      post({ rightType: "erasure", subjectIdentifier: "member-alice", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(403);
    const body = (await response.json()) as { data: null; meta: { refusal: string } };
    expect(body.meta.refusal).toBe("sessions.membership_required");
    const audit = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query("SELECT request_id FROM session_subject_audit"),
    );
    expect(audit.rows).toHaveLength(0);
  });

  test("a participant requesting access is 403 (export not granted)", async () => {
    const handler = makeHandler("participant");
    const response = await handler(
      post({ rightType: "access", subjectIdentifier: "member-alice", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(403);
  });

  test("the tenant in the body must be the principal's tenant: cross-tenant is 403 with zero writes", async () => {
    // An owner authenticated for tenant B must not operate tenant A, whatever
    // the body claims — the K2 authoritative tenant is the principal's.
    const handler = makeHandler("owner", "ten_bbbbbbbbbbbbbbbb");
    const response = await handler(
      post({ rightType: "erasure", subjectIdentifier: "member-alice", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(403);
    const body = (await response.json()) as { data: null; meta: { refusal: string } };
    expect(body.meta.refusal).toBe("sessions.membership_required");
    const audit = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query("SELECT request_id FROM session_subject_audit"),
    );
    expect(audit.rows).toHaveLength(0);
    const tombstones = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query("SELECT subject_digest FROM session_deleted_subjects"),
    );
    expect(tombstones.rows).toHaveLength(0);
  });

  test("a facilitator requesting restriction is 403 (restriction maps to delete, owner-only)", async () => {
    // Restriction is OPERATION_BY_RIGHT-mapped to "delete": the facilitator
    // role holds "export" but not "delete" (ROLE_OPERATIONS), same
    // deny-by-default gate as erasure above, before any I/O.
    const handler = makeHandler("facilitator");
    const response = await handler(
      post({
        rightType: "restriction",
        subjectIdentifier: "member-alice",
        tenantId: TENANT_A,
        ground: "accuracy-contested",
      }),
    );
    expect(response.status).toBe(403);
    const body = (await response.json()) as { data: null; meta: { refusal: string } };
    expect(body.meta.refusal).toBe("sessions.membership_required");
    const audit = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query("SELECT request_id FROM session_subject_audit"),
    );
    expect(audit.rows).toHaveLength(0);
  });
});

describe("verification", () => {
  test("an unverifiable subject is 404 with a refused audit row", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "access", subjectIdentifier: "member-ghost", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(404);
    const body = (await response.json()) as {
      data: { request: { requestId: string; status: string } } | null;
      meta: { refusal: string };
    };
    expect(body.meta.refusal).toBe("sessions.rgpd.subject_unverified");
    expect(body.data?.request.status).toBe("refused");
    const audit = await auditRows(body.data?.request.requestId ?? "");
    expect(audit.rows).toEqual([
      { status: "refused", detail: "sessions.rgpd.subject_unverified", receipt_id: null },
    ]);
  });
});

describe("fulfilled flows", () => {
  test("access: 200 with the export and received+fulfilled audit rows", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "access", subjectIdentifier: "owner-alpha", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: {
        request: { requestId: string; status: string; deadline: string };
        result: { status: string; dataExport: { events: unknown[] } };
      };
      meta: Record<string, never>;
    };
    expect(body.data.request.status).toBe("fulfilled");
    expect(body.data.request.deadline).toBe("2026-08-22T10:00:00.000Z");
    expect(body.data.result.status).toBe("fulfilled");
    expect(body.data.result.dataExport.events).toHaveLength(1);
    const audit = await auditRows(body.data.request.requestId);
    expect(audit.rows).toEqual([
      { status: "fulfilled", detail: null, receipt_id: null },
      { status: "received", detail: null, receipt_id: null },
    ]);
  });

  test("erasure: 200 with receipt id, tombstone persisted, full audit trail", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "erasure", subjectIdentifier: "member-alice", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: {
        request: { requestId: string; status: string };
        result: { status: string; deletionReceiptId: string; recordsAffected: number };
      };
    };
    expect(body.data.result.status).toBe("fulfilled");
    expect(body.data.result.recordsAffected).toBe(1);
    const tombstones = await withTenantDbTransaction(tdb.db, TENANT_A, (tx) =>
      tx.query<{ receipt_id: string }>("SELECT receipt_id FROM session_deleted_subjects"),
    );
    expect(tombstones.rows).toEqual([{ receipt_id: body.data.result.deletionReceiptId }]);
    // The audit trail joins the deletion evidence at the storage level: the
    // terminal row carries the receipt id an auditor cross-references.
    const audit = await auditRows(body.data.request.requestId);
    expect(audit.rows).toEqual([
      { status: "fulfilled", detail: null, receipt_id: body.data.result.deletionReceiptId },
      { status: "received", detail: null, receipt_id: null },
    ]);
  });

  test("a deferred right is a typed refusal with a complete audit trail", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "rectification", subjectIdentifier: "owner-alpha", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: { request: { requestId: string; status: string } };
      meta: { refusal: string };
    };
    expect(body.meta.refusal).toBe("sessions.rgpd.not_implemented");
    const audit = await auditRows(body.data.request.requestId);
    expect(audit.rows).toEqual([
      { status: "received", detail: null, receipt_id: null },
      { status: "refused", detail: "sessions.rgpd.not_implemented", receipt_id: null },
    ]);
  });
});

describe("restriction ground intake and fulfillment", () => {
  test("restriction without a ground is 400 request_invalid", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "restriction", subjectIdentifier: "owner-alpha", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(400);
    const body = (await response.json()) as { data: null; meta: { refusal: string } };
    expect(body.meta.refusal).toBe("sessions.rgpd.request_invalid");
  });

  test("restriction with an invalid ground value is 400 request_invalid", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({
        rightType: "restriction",
        subjectIdentifier: "owner-alpha",
        tenantId: TENANT_A,
        ground: "not-a-real-ground",
      }),
    );
    expect(response.status).toBe(400);
    const body = (await response.json()) as { data: null; meta: { refusal: string } };
    expect(body.meta.refusal).toBe("sessions.rgpd.request_invalid");
  });

  test("a non-restriction body carrying a ground is 400 request_invalid", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({
        rightType: "access",
        subjectIdentifier: "owner-alpha",
        tenantId: TENANT_A,
        ground: "accuracy-contested",
      }),
    );
    expect(response.status).toBe(400);
    const body = (await response.json()) as { data: null; meta: { refusal: string } };
    expect(body.meta.refusal).toBe("sessions.rgpd.request_invalid");
  });

  test("restriction with a valid ground restricts and returns fulfilled with a full audit trail", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({
        rightType: "restriction",
        subjectIdentifier: "owner-alpha",
        tenantId: TENANT_A,
        ground: "accuracy-contested",
      }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: {
        request: { requestId: string; status: string };
        result: { status: string; ground: string; affectedRecords: number };
      };
      meta: Record<string, never>;
    };
    expect(body.data.request.status).toBe("fulfilled");
    expect(body.data.result.status).toBe("fulfilled");
    // The Art. 18(1) ground rides back on the fulfillment, unchanged.
    expect(body.data.result.ground).toBe("accuracy-contested");
    const audit = await auditRows(body.data.request.requestId);
    expect(audit.rows).toEqual([
      { status: "fulfilled", detail: null, receipt_id: null },
      { status: "received", detail: null, receipt_id: null },
    ]);
  });

  test("restricting an already-restricted subject is refused, with a received+refused audit trail", async () => {
    // owner-alpha is restricted by the test above (state = highest entry_seq
    // in session_restricted_subjects) — a second restriction request through
    // the SAME handler hits the ladder's already_restricted rung.
    const handler = makeHandler("owner");
    const response = await handler(
      post({
        rightType: "restriction",
        subjectIdentifier: "owner-alpha",
        tenantId: TENANT_A,
        ground: "objection-pending",
      }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: { request: { requestId: string; status: string } };
      meta: { refusal: string };
    };
    expect(body.meta.refusal).toBe("sessions.rgpd.already_restricted");
    expect(body.data.request.status).toBe("refused");
    const audit = await auditRows(body.data.request.requestId);
    expect(audit.rows).toEqual([
      { status: "received", detail: null, receipt_id: null },
      { status: "refused", detail: "sessions.rgpd.already_restricted", receipt_id: null },
    ]);
  });
});

describe("§6.3: restriction pauses disclosure, never the subject's own rights", () => {
  // owner-alpha is still restricted here (the describe block above leaves it
  // so). Art. 18(2): the subject's own access (Art. 15), portability
  // (Art. 20) and erasure (Art. 17) requests are NOT gated by isRestricted —
  // only serving contributions to OTHERS, exports, synthesis and retention
  // sweeps pause. Exercised end to end through the SAME handler used for the
  // restriction request above, not just at the port level.
  //
  // Ordering dependency: the last test below erases owner-alpha, the shared
  // fixture actor, and erasure is irreversible. This MUST stay the LAST
  // describe block in the file — any new, non-destructive test belongs
  // ABOVE it, never appended after.
  test("access is still fulfilled for a restricted subject", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "access", subjectIdentifier: "owner-alpha", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: { result: { status: string } };
      meta: Record<string, never>;
    };
    expect(body.data.result.status).toBe("fulfilled");
  });

  test("portability is still fulfilled for a restricted subject", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "portability", subjectIdentifier: "owner-alpha", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: { result: { status: string } };
      meta: Record<string, never>;
    };
    expect(body.data.result.status).toBe("fulfilled");
  });

  test("erasure is still fulfilled for a restricted subject (erasure supersedes restriction)", async () => {
    const handler = makeHandler("owner");
    const response = await handler(
      post({ rightType: "erasure", subjectIdentifier: "owner-alpha", tenantId: TENANT_A }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      data: { result: { status: string; deletionReceiptId: string } };
    };
    expect(body.data.result.status).toBe("fulfilled");
    expect(body.data.result.deletionReceiptId).toBeTruthy();
  });
});
