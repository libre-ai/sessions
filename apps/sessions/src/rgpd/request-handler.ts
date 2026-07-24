// The Sessions data-subject request handler (design §5 step 5), an exported
// factory deliberately NOT mounted on the public cockpit routes: the Sessions
// runtime boundary is locked until WP-G3-S01's sessions-authz-review human
// gate approves real transport integration (see createSessionsHandler's
// fixture-only comment). Wiring it later is one route entry; the whole flow
// is integration-tested here in the meantime.
//
// Order of gates, fail-closed and cheapest-first, before anything touches
// the database: method → body shape → deny-by-default authorization (the
// caller's principal against the locked sessions operation matrix) → subject
// verification. Only then is the request recorded (`received`) and
// dispatched to the port; the terminal state (`fulfilled`/`refused`) is
// appended to the same per-context audit trail. Envelope: { data, meta }.
//
// Two request-id families coexist by design: `request.requestId` anchors the
// API request and its audit rows; an erasure result's `deletionReceiptId` is
// the port-generated deletion-transaction id, cross-referenced by the
// tombstone's receipt_id AND persisted on the terminal audit row
// (`receipt_id` column), so audit trail and deletion evidence stay joined at
// the storage level. The audit `detail` column carries refusal codes only —
// never free text, never PII.

import { type SqlExecutor, withTenantDbTransaction } from "@libre-ai/data";
import {
  type AccessRequestResult,
  computeResponseDeadline,
  DATA_SUBJECT_RIGHT_TYPES,
  type DataSubjectRequest,
  type DataSubjectRightsPort,
  type DataSubjectRightType,
  deriveSubjectDigest,
  type ErasureRequestResult,
  InvalidSubjectIdentifierError,
  isRestrictionGround,
  type PortabilityRequestResult,
  type RestrictionGround,
  type RestrictionRequestResult,
  validateDataSubjectRequest,
} from "@libre-ai/rgpd-kit";
import {
  roleHasOperation,
  type SessionOperation,
  type SessionRole,
} from "../authz/session-authorization";

/**
 * The authenticated caller, bound to the ONE tenant its authentication is
 * for (K2: the authoritative tenant comes from the verified principal, never
 * from the request body). A role alone is not an authorization — the same
 * role in another tenant grants nothing here.
 */
export interface RgpdPrincipal {
  readonly role: SessionRole;
  readonly tenantId: string;
}

export interface DataSubjectRequestDeps {
  readonly port: DataSubjectRightsPort;
  readonly executor: SqlExecutor;
  /** Pre-authenticated caller — authorization here is deny-by-default. */
  readonly principal: RgpdPrincipal;
  readonly now: () => string;
  readonly newRequestId: () => string;
}

// Which locked sessions operation a right requires (deny-by-default against
// ROLE_OPERATIONS): reading data out needs `export`, every mutating or
// state-affecting right needs `delete`. Observers hold neither.
const OPERATION_BY_RIGHT: Readonly<Record<DataSubjectRightType, SessionOperation>> = {
  access: "export",
  portability: "export",
  erasure: "delete",
  restriction: "delete",
  rectification: "delete",
  object: "delete",
};

const PRIVATE_TENANT_ID = /^ten_[a-z0-9]{16,64}$/;

type PortResult =
  | AccessRequestResult
  | ErasureRequestResult
  | RestrictionRequestResult
  | PortabilityRequestResult;

interface NonRestrictionParsedBody {
  readonly rightType: Exclude<DataSubjectRightType, "restriction">;
  readonly subjectIdentifier: string;
  readonly tenantId: string;
}

interface RestrictionParsedBody {
  readonly rightType: "restriction";
  readonly subjectIdentifier: string;
  readonly tenantId: string;
  // Art. 18(1) ground: belongs to the subject, enters through the request
  // (rgpd-kit port contract), never invented by this handler.
  readonly ground: RestrictionGround;
}

type ParsedBody = NonRestrictionParsedBody | RestrictionParsedBody;

function envelope(data: unknown, meta: Record<string, unknown>, status: number): Response {
  return Response.json({ data, meta }, { status });
}

function parseBody(raw: unknown): ParsedBody | null {
  if (typeof raw !== "object" || raw === null) {
    return null;
  }
  const candidate = raw as Record<string, unknown>;
  if (
    typeof candidate.rightType !== "string" ||
    !(DATA_SUBJECT_RIGHT_TYPES as readonly string[]).includes(candidate.rightType)
  ) {
    return null;
  }
  if (typeof candidate.subjectIdentifier !== "string" || candidate.subjectIdentifier === "") {
    return null;
  }
  if (typeof candidate.tenantId !== "string" || !PRIVATE_TENANT_ID.test(candidate.tenantId)) {
    return null;
  }
  const rightType = candidate.rightType as DataSubjectRightType;
  // The Art. 18(1) ground belongs to the subject and must enter through the
  // request: required (and validated) on restriction, and rejected on every
  // other right rather than silently ignored.
  if (rightType === "restriction") {
    if (!isRestrictionGround(candidate.ground)) {
      return null;
    }
    return {
      rightType,
      subjectIdentifier: candidate.subjectIdentifier,
      tenantId: candidate.tenantId,
      ground: candidate.ground,
    };
  }
  if (candidate.ground !== undefined) {
    return null;
  }
  return {
    rightType,
    subjectIdentifier: candidate.subjectIdentifier,
    tenantId: candidate.tenantId,
  };
}

export function createDataSubjectRequestHandler(
  deps: DataSubjectRequestDeps,
): (request: Request) => Promise<Response> {
  async function appendAudit(
    request: DataSubjectRequest,
    status: "received" | "fulfilled" | "refused",
    detail: string | null,
    receiptId: string | null = null,
  ): Promise<void> {
    await withTenantDbTransaction(deps.executor, request.tenantId, async (tx) => {
      await tx.query(
        `INSERT INTO session_subject_audit
           (tenant_id, request_id, subject_digest, right_type, status, detail, receipt_id, recorded_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)`,
        [
          request.tenantId,
          request.requestId,
          request.subjectDigest,
          request.rightType,
          status,
          detail,
          receiptId,
          deps.now(),
        ],
      );
    });
  }

  async function dispatch(
    body: ParsedBody,
    subjectDigest: string,
    requestId: string,
  ): Promise<PortResult> {
    switch (body.rightType) {
      case "access":
        return deps.port.handleAccessRequest(body.tenantId, subjectDigest);
      case "erasure":
        return deps.port.handleErasureRequest(body.tenantId, subjectDigest);
      case "restriction":
        return deps.port.handleRestrictionRequest(body.tenantId, subjectDigest, body.ground);
      case "portability":
        return deps.port.handlePortabilityRequest(body.tenantId, subjectDigest);
      case "rectification":
      case "object":
        // No port surface yet (design §6): a typed refusal under the SAME
        // request id as the audit trail, still audited.
        return { status: "refused", requestId, refusal: "sessions.rgpd.not_implemented" };
    }
  }

  return async (request: Request): Promise<Response> => {
    if (request.method !== "POST") {
      return envelope(null, { refusal: "sessions.rgpd.method_not_allowed" }, 405);
    }
    let rawBody: unknown;
    try {
      rawBody = await request.json();
    } catch {
      return envelope(null, { refusal: "sessions.rgpd.request_invalid" }, 400);
    }
    const body = parseBody(rawBody);
    if (body === null) {
      return envelope(null, { refusal: "sessions.rgpd.request_invalid" }, 400);
    }

    // Deny-by-default, before any I/O: the principal must hold the locked
    // sessions operation the right maps to, AND the body's tenant must be
    // the principal's own tenant — the body never chooses the tenant scope
    // (K4 finding: otherwise any authenticated owner could export or erase
    // inside a foreign tenant, with RLS keyed on the attacker's value).
    if (!roleHasOperation(deps.principal.role, OPERATION_BY_RIGHT[body.rightType])) {
      return envelope(null, { refusal: "sessions.membership_required" }, 403);
    }
    if (body.tenantId !== deps.principal.tenantId) {
      return envelope(null, { refusal: "sessions.membership_required" }, 403);
    }

    let subjectDigest: string;
    try {
      subjectDigest = await deriveSubjectDigest(body.tenantId, body.subjectIdentifier);
    } catch (error) {
      if (error instanceof InvalidSubjectIdentifierError) {
        return envelope(null, { refusal: "sessions.rgpd.request_invalid" }, 400);
      }
      throw error;
    }

    const receivedAt = deps.now();
    const base = {
      requestId: deps.newRequestId(),
      subjectDigest,
      rightType: body.rightType,
      tenantId: body.tenantId,
      receivedAt,
      submittedVia: "api",
      deadline: computeResponseDeadline(receivedAt),
    };

    const verified = await deps.port.verifySubject(body.tenantId, body.subjectIdentifier);
    if (verified === null) {
      const refused = validateDataSubjectRequest({
        ...base,
        status: "refused",
        refusalReason: "sessions.rgpd.subject_unverified",
      });
      await appendAudit(refused, "refused", "sessions.rgpd.subject_unverified");
      return envelope({ request: refused }, { refusal: "sessions.rgpd.subject_unverified" }, 404);
    }

    const received = validateDataSubjectRequest({ ...base, status: "received" });
    await appendAudit(received, "received", null);

    const result = await dispatch(body, verified, base.requestId);
    if (result.status === "refused") {
      const refused = validateDataSubjectRequest({
        ...base,
        status: "refused",
        refusalReason: result.refusal,
      });
      await appendAudit(refused, "refused", result.refusal);
      return envelope({ request: refused, result }, { refusal: result.refusal }, 200);
    }
    const fulfilled = validateDataSubjectRequest({ ...base, status: "fulfilled" });
    const receiptId = "deletionReceiptId" in result ? result.deletionReceiptId : null;
    await appendAudit(fulfilled, "fulfilled", null, receiptId);
    return envelope({ request: fulfilled, result }, {}, 200);
  };
}
