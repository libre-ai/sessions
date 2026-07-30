// Sessions command service — composes the vertical for one action inside the
// caller's tenant transaction (packages/data withTenantDbTransaction): validate
// the untrusted event, authorize the actor's role against the locked policy, fold
// it onto the persisted stream, then append it. Fail-closed and ordered so the
// cheapest, most fundamental checks run first and nothing touches the log until
// the action is both well-formed and authorized.

import type { SqlExecutor } from "@libre-ai/data";
import {
  type AuthzRefusalCode,
  authorizeAction,
  type SessionRole,
} from "../authz/session-authorization";
import {
  type RefusalCode as DomainRefusalCode,
  reduce,
  type SessionEvent,
  type SessionState,
  validateEvent,
} from "../domain/session-event";
import {
  appendEvent,
  loadSessionState,
  SessionSequenceConflictError,
  SessionTenantMismatchError,
} from "../persistence/session-event-store";

export interface SessionPrincipal {
  readonly role: SessionRole;
}

export type SessionCommandRefusal = DomainRefusalCode | AuthzRefusalCode;

export type SessionCommandResult =
  | { readonly status: "accepted"; readonly event: SessionEvent; readonly state: SessionState }
  | { readonly status: "refused"; readonly refusal: SessionCommandRefusal };

function refused(refusal: SessionCommandRefusal): SessionCommandResult {
  return { status: "refused", refusal };
}

/**
 * Execute one session action (an appended event) end to end, within the caller's
 * tenant transaction. In order: `validateEvent` (structure → cursor_invalid),
 * `authorizeAction` (role → membership_required, before any I/O), fold onto the
 * loaded stream via `reduce` (ordering/identity/revision → cursor_invalid /
 * tenant_mismatch / revision_stale), then `appendEvent`. A concurrent writer that
 * takes the same sequence between the load and the append is surfaced as
 * `sessions.cursor_invalid` (the stream advanced past the append point); a
 * foreign-tenant first event — which `reduce` cannot catch with no prior state to
 * compare against — is stopped at the append barrier and surfaced as
 * `sessions.tenant_mismatch`.
 */
export async function executeSessionCommand(
  executor: SqlExecutor,
  principal: SessionPrincipal,
  rawEvent: unknown,
  recordedAt: string,
): Promise<SessionCommandResult> {
  const validated = validateEvent(rawEvent);
  if (!validated.ok) return refused(validated.refusal);
  const event = validated.value;

  const authorized = authorizeAction(principal.role, event.type);
  if (!authorized.ok) return refused(authorized.refusal);

  const state = await loadSessionState(executor, event.sessionId);
  const step = reduce(state, event);
  if (!step.ok) return refused(step.refusal);

  try {
    await appendEvent(executor, event, recordedAt);
  } catch (error) {
    if (error instanceof SessionSequenceConflictError) return refused("sessions.cursor_invalid");
    if (error instanceof SessionTenantMismatchError) return refused("sessions.tenant_mismatch");
    throw error;
  }
  return { status: "accepted", event, state: step.value };
}
