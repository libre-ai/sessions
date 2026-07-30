// Sessions domain — the pure, fail-closed validator and append-only reducer for
// the session event stream (docs/apps/sessions.md §Domain protocol; contracts/
// schemas/session-event.v1.schema.json). This module does not persist, does not
// transmit, and imports nothing. `validateEvent` reconstructs a typed, contract-
// conformant event from untrusted input; `reduce` folds a validated event onto
// the accumulated session state, enforcing the append-only invariants: a stream
// opens with session-created, sequence is strictly contiguous, revision never
// rewinds, tenant and session identity are consistent, and nothing follows a
// deletion. Downstream persistence (RLS, constraints) enforces these
// structurally; this validates the semantic preconditions.

// Patterns are the LOCKED common.v1 definitions the schema $refs — reused
// verbatim, not reinvented (common.v1.schema.json#/$defs).
const URN = /^urn:libre-ai:[a-z][a-z0-9-]*:[A-Za-z0-9._~-]+$/;
const TENANT_ID = /^ten_[a-z0-9]{16,64}$/;
const IDENTIFIER = /^[a-z][a-z0-9_-]{2,127}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const MEDIA_TYPE = /^[a-z0-9!#$&^_.+-]+\/[a-z0-9!#$&^_.+-]+$/;
const REASON_CODE = /^sessions\.[a-z0-9_.-]+$/;
// RFC 3339 date-time (common.v1 timestamp is `format: date-time`): a UTC/offset
// designator is required, and time-secfrac is 1+ digits.
const TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;

export const EVENT_TYPES = [
  "member-added",
  "session-created",
  "source-attached",
  "participant-joined",
  "contribution-submitted",
  "synthesis-drafted",
  "outcome-approved",
  "outcome-rejected",
  "session-closed",
  "session-exported",
  "session-deleted",
] as const;
export type EventType = (typeof EVENT_TYPES)[number];

const ACTOR_KINDS = ["human", "provider", "system"] as const;
export type ActorKind = (typeof ACTOR_KINDS)[number];

const AUDIENCES = ["private", "facilitators", "session"] as const;
export type Audience = (typeof AUDIENCES)[number];

export interface Actor {
  readonly kind: ActorKind;
  readonly id: string;
}

export interface ArtifactReference {
  readonly id: string;
  readonly digest: string;
  readonly mediaType: string;
}

export interface EventData {
  readonly resourceId?: string;
  readonly audience?: Audience;
  readonly contentDigest?: string;
  readonly reasonCode?: string;
  readonly artifact?: ArtifactReference;
}

export interface SessionEvent {
  readonly schemaVersion: "libre-ai.session-event.v1";
  readonly id: string;
  readonly tenantId: string;
  readonly sessionId: string;
  readonly sequence: number;
  readonly revision: number;
  readonly type: EventType;
  readonly actor: Actor;
  readonly occurredAt: string;
  readonly data: EventData;
}

export interface SessionState {
  readonly tenantId: string;
  readonly sessionId: string;
  readonly closed: boolean;
  readonly exported: boolean;
  readonly deleted: boolean;
  readonly latestSequence: number;
  readonly latestRevision: number;
  readonly eventCount: number;
}

export type RefusalCode =
  | "sessions.cursor_invalid"
  | "sessions.tenant_mismatch"
  | "sessions.revision_stale";

export type Outcome<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly refusal: RefusalCode };

function invalid<T>(): Outcome<T> {
  return { ok: false, refusal: "sessions.cursor_invalid" };
}

// --- structural validation --------------------------------------------------

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(obj: Record<string, unknown>, allowed: readonly string[]): boolean {
  const permitted = new Set(allowed);
  return Object.keys(obj).every((key) => permitted.has(key));
}

function validActor(value: unknown): Actor | undefined {
  if (!isObject(value) || !hasExactKeys(value, ["kind", "id"])) return undefined;
  const { kind, id } = value;
  if (typeof kind !== "string" || !(ACTOR_KINDS as readonly string[]).includes(kind))
    return undefined;
  if (typeof id !== "string" || !IDENTIFIER.test(id)) return undefined;
  return { kind: kind as ActorKind, id };
}

function validArtifact(value: unknown): ArtifactReference | undefined {
  if (!isObject(value) || !hasExactKeys(value, ["id", "digest", "mediaType"])) return undefined;
  const { id, digest, mediaType } = value;
  if (typeof id !== "string" || !URN.test(id)) return undefined;
  if (typeof digest !== "string" || !SHA256.test(digest)) return undefined;
  if (typeof mediaType !== "string" || !MEDIA_TYPE.test(mediaType)) return undefined;
  // Deep-freeze: a shallow freeze of `data` would leave the nested artifact
  // mutable, letting a caller forge its digest/mediaType after validation.
  return Object.freeze({ id, digest, mediaType });
}

const DATA_KEYS = ["resourceId", "audience", "contentDigest", "reasonCode", "artifact"] as const;

function validData(value: unknown): EventData | undefined {
  if (!isObject(value) || !hasExactKeys(value, DATA_KEYS)) return undefined;
  const data: {
    resourceId?: string;
    audience?: Audience;
    contentDigest?: string;
    reasonCode?: string;
    artifact?: ArtifactReference;
  } = {};
  if (value.resourceId !== undefined) {
    if (typeof value.resourceId !== "string" || !URN.test(value.resourceId)) return undefined;
    data.resourceId = value.resourceId;
  }
  if (value.audience !== undefined) {
    if (
      typeof value.audience !== "string" ||
      !(AUDIENCES as readonly string[]).includes(value.audience)
    ) {
      return undefined;
    }
    data.audience = value.audience as Audience;
  }
  if (value.contentDigest !== undefined) {
    if (typeof value.contentDigest !== "string" || !SHA256.test(value.contentDigest))
      return undefined;
    data.contentDigest = value.contentDigest;
  }
  if (value.reasonCode !== undefined) {
    if (typeof value.reasonCode !== "string" || !REASON_CODE.test(value.reasonCode))
      return undefined;
    data.reasonCode = value.reasonCode;
  }
  if (value.artifact !== undefined) {
    const artifact = validArtifact(value.artifact);
    if (artifact === undefined) return undefined;
    data.artifact = artifact;
  }
  return data;
}

// Per-type conditional requirements from the schema's allOf/if-then.
function dataSatisfiesType(type: EventType, data: EventData): boolean {
  if (type === "contribution-submitted") {
    return (
      data.resourceId !== undefined &&
      data.audience !== undefined &&
      data.contentDigest !== undefined
    );
  }
  if (type === "synthesis-drafted" || type === "outcome-approved") {
    return data.artifact !== undefined;
  }
  return true;
}

const EVENT_KEYS = [
  "schemaVersion",
  "id",
  "tenantId",
  "sessionId",
  "sequence",
  "revision",
  "type",
  "actor",
  "occurredAt",
  "data",
] as const;

function isInteger(value: unknown, min: number): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= min;
}

/**
 * Validate untrusted input into a typed, contract-conformant session event.
 * Fail-closed: any structural or format violation — unknown keys, malformed URN,
 * non-`ten_` tenant, out-of-range sequence/revision, bad timestamp, or a data
 * payload that does not satisfy the event type's requirements — is refused as
 * `sessions.cursor_invalid`.
 */
export function validateEvent(input: unknown): Outcome<SessionEvent> {
  if (!isObject(input) || !hasExactKeys(input, EVENT_KEYS)) return invalid();
  for (const key of EVENT_KEYS) {
    if (!(key in input)) return invalid();
  }
  if (input.schemaVersion !== "libre-ai.session-event.v1") return invalid();
  if (typeof input.id !== "string" || !URN.test(input.id)) return invalid();
  if (typeof input.tenantId !== "string" || !TENANT_ID.test(input.tenantId)) return invalid();
  if (typeof input.sessionId !== "string" || !URN.test(input.sessionId)) return invalid();
  if (!isInteger(input.sequence, 1)) return invalid();
  if (!isInteger(input.revision, 0)) return invalid();
  if (typeof input.type !== "string" || !(EVENT_TYPES as readonly string[]).includes(input.type)) {
    return invalid();
  }
  const actor = validActor(input.actor);
  if (actor === undefined) return invalid();
  if (typeof input.occurredAt !== "string" || !TIMESTAMP.test(input.occurredAt)) return invalid();
  if (Number.isNaN(new Date(input.occurredAt).getTime())) return invalid();
  const data = validData(input.data);
  if (data === undefined) return invalid();
  const type = input.type as EventType;
  if (!dataSatisfiesType(type, data)) return invalid();

  return {
    ok: true,
    value: Object.freeze({
      schemaVersion: "libre-ai.session-event.v1",
      id: input.id,
      tenantId: input.tenantId,
      sessionId: input.sessionId,
      sequence: input.sequence,
      revision: input.revision,
      type,
      actor: Object.freeze(actor),
      occurredAt: input.occurredAt,
      data: Object.freeze(data),
    }),
  };
}

// --- append-only reduction --------------------------------------------------

/**
 * Fold a validated event onto the accumulated session state. Pass `null` for the
 * first event of a stream, which must be `session-created`. Enforces the
 * append-only invariants:
 * - tenant identity is consistent (`sessions.tenant_mismatch`);
 * - session identity, contiguous strictly-increasing sequence, a single
 *   creation, and no event after deletion (`sessions.cursor_invalid`);
 * - revision never rewinds (`sessions.revision_stale`).
 */
export function reduce(state: SessionState | null, event: SessionEvent): Outcome<SessionState> {
  if (state === null) {
    if (event.type !== "session-created" || event.sequence !== 1) return invalid();
    return { ok: true, value: freezeState(build(event)) };
  }
  if (event.tenantId !== state.tenantId) return { ok: false, refusal: "sessions.tenant_mismatch" };
  if (event.sessionId !== state.sessionId) return invalid();
  if (state.deleted) return invalid();
  if (event.type === "session-created") return invalid();
  if (event.sequence !== state.latestSequence + 1) return invalid();
  if (event.revision < state.latestRevision)
    return { ok: false, refusal: "sessions.revision_stale" };

  return { ok: true, value: freezeState(apply(state, event)) };
}

function build(event: SessionEvent): SessionState {
  return {
    tenantId: event.tenantId,
    sessionId: event.sessionId,
    closed: false,
    exported: false,
    deleted: false,
    latestSequence: event.sequence,
    latestRevision: event.revision,
    eventCount: 1,
  };
}

function apply(state: SessionState, event: SessionEvent): SessionState {
  return {
    tenantId: state.tenantId,
    sessionId: state.sessionId,
    closed: state.closed || event.type === "session-closed",
    exported: state.exported || event.type === "session-exported",
    deleted: state.deleted || event.type === "session-deleted",
    latestSequence: event.sequence,
    latestRevision: event.revision,
    eventCount: state.eventCount + 1,
  };
}

function freezeState(state: SessionState): SessionState {
  return Object.freeze(state);
}
