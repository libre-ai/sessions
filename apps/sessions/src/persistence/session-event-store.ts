// Sessions v1 persistence adapter. Appends validated domain events to the
// tenant-scoped, append-only log inside the caller's tenant transaction
// (packages/data withTenantDbTransaction): the tenant is read from the active
// context (never the request), FORCE RLS scopes every row, and the composite key
// makes a replayed sequence conflict rather than fork. State is rebuilt by
// folding the persisted stream back through the domain reducer.

import { requireTenantContext, type SqlExecutor } from "@libre-ai/data";
import {
  type ActorKind,
  type EventType,
  reduce,
  type SessionEvent,
  type SessionState,
} from "../domain/session-event";

export class SessionSequenceConflictError extends Error {
  constructor(
    readonly sessionId: string,
    readonly sequence: number,
  ) {
    super(`session event sequence conflict for ${sessionId}@${sequence}`);
    this.name = "SessionSequenceConflictError";
  }
}

export class SessionTenantMismatchError extends Error {
  constructor() {
    super("event tenant differs from the active tenant context");
    this.name = "SessionTenantMismatchError";
  }
}

export class SessionStreamCorruptError extends Error {
  constructor(readonly sessionId: string) {
    super(`persisted session stream does not reduce for ${sessionId}`);
    this.name = "SessionStreamCorruptError";
  }
}

interface SessionEventRow {
  readonly tenant_id: string;
  readonly session_id: string;
  readonly sequence: number;
  readonly event_id: string;
  readonly revision: number;
  readonly type: string;
  readonly actor_kind: string;
  readonly actor_id: string;
  readonly occurred_at: string | Date;
  readonly data: unknown;
}

function asJson(value: unknown): unknown {
  return typeof value === "string" ? JSON.parse(value) : value;
}

function asIsoString(value: string | Date): string {
  return value instanceof Date ? value.toISOString() : value;
}

function rowToEvent(row: SessionEventRow): SessionEvent {
  return {
    schemaVersion: "libre-ai.session-event.v1",
    id: row.event_id,
    tenantId: row.tenant_id,
    sessionId: row.session_id,
    sequence: row.sequence,
    revision: row.revision,
    type: row.type as EventType,
    actor: { kind: row.actor_kind as ActorKind, id: row.actor_id },
    occurredAt: asIsoString(row.occurred_at),
    data: asJson(row.data) as SessionEvent["data"],
  };
}

/**
 * Append one validated event to the log, within the caller's tenant transaction.
 * Fail-closed: the event's tenant must match the active context (defense in depth
 * above RLS), and a duplicate `(tenant, session, sequence)` — a replay — inserts
 * no row and throws `SessionSequenceConflictError` rather than forking the stream.
 */
export async function appendEvent(
  executor: SqlExecutor,
  event: SessionEvent,
  recordedAt: string,
): Promise<void> {
  const tenantId = requireTenantContext();
  if (event.tenantId !== tenantId) throw new SessionTenantMismatchError();

  const result = await executor.query(
    `INSERT INTO session_events (
       tenant_id, session_id, sequence, event_id, revision, type,
       actor_kind, actor_id, occurred_at, data, recorded_at
     ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
     ON CONFLICT (tenant_id, session_id, sequence) DO NOTHING`,
    [
      tenantId,
      event.sessionId,
      event.sequence,
      event.id,
      event.revision,
      event.type,
      event.actor.kind,
      event.actor.id,
      event.occurredAt,
      JSON.stringify(event.data),
      recordedAt,
    ],
  );
  if ((result.affectedRows ?? 0) !== 1)
    throw new SessionSequenceConflictError(event.sessionId, event.sequence);
}

/**
 * Load a session's events in causal order. Tenant scoping is by RLS: the read
 * runs under the active tenant context, so a foreign-tenant session simply
 * returns no rows (never another tenant's stream).
 */
export async function loadEvents(
  executor: SqlExecutor,
  sessionId: string,
): Promise<SessionEvent[]> {
  const { rows } = await executor.query<SessionEventRow>(
    "SELECT * FROM session_events WHERE session_id = $1 ORDER BY sequence ASC",
    [sessionId],
  );
  return rows.map(rowToEvent);
}

/**
 * Rebuild a session's state by folding its persisted stream through the domain
 * reducer. Returns `null` for an unknown/empty session. A persisted stream that
 * does not reduce is a corruption — surfaced as `SessionStreamCorruptError`, not
 * silently ignored. The reducer checks the STRUCTURAL invariants (sequence,
 * tenant/session identity, revision, ordering); field-level integrity of the
 * `data` payload is the write-time `validateEvent` guarantee, not re-checked here.
 */
export async function loadSessionState(
  executor: SqlExecutor,
  sessionId: string,
): Promise<SessionState | null> {
  const events = await loadEvents(executor, sessionId);
  if (events.length === 0) return null;
  let state: SessionState | null = null;
  for (const event of events) {
    const step = reduce(state, event);
    if (!step.ok) throw new SessionStreamCorruptError(sessionId);
    state = step.value;
  }
  return state;
}
