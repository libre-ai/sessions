// Sessions app-side authorization matrix, conformant to the LOCKED authorizer
// policy contracts/authz/sessions-v1.datalog. This is the deny-by-default
// decision the Biscuit authorizer encodes, mirrored in TypeScript so the app
// refuses before touching state; the datalog remains the source of truth (a
// conformance test parses it and fails if this matrix drifts). Tenant scoping
// (resource_tenant) is enforced structurally by RLS and the reducer, not here.

import type { Audience, EventType } from "../domain/session-event";

export type SessionRole = "owner" | "facilitator" | "participant" | "observer";

// The operation vocabulary from the datalog's allow rules.
export type SessionOperation =
  | "create-space"
  | "add-member"
  | "create-session"
  | "set-audience"
  | "attach-source"
  | "join"
  | "submit"
  | "draft"
  | "approve"
  | "reject"
  | "close"
  | "export"
  | "delete"
  | "read";

export type AuthzRefusalCode = "sessions.membership_required" | "sessions.audience_forbidden";

export type AuthzOutcome =
  | { readonly ok: true }
  | { readonly ok: false; readonly refusal: AuthzRefusalCode };

const ALLOWED: AuthzOutcome = { ok: true };

// Each mutating event is the record of an operation; producing it requires that
// operation. `read` is not an event (it is a query), so it is absent here and
// handled by `authorizeRead`.
export const EVENT_OPERATION: Readonly<Record<EventType, SessionOperation>> = {
  "member-added": "add-member",
  "session-created": "create-session",
  "source-attached": "attach-source",
  "participant-joined": "join",
  "contribution-submitted": "submit",
  "synthesis-drafted": "draft",
  "outcome-approved": "approve",
  "outcome-rejected": "reject",
  "session-closed": "close",
  "session-exported": "export",
  "session-deleted": "delete",
};

// The UNCONDITIONAL operation grant per role, verbatim from sessions-v1.datalog.
// The participant's and observer's audience-conditional reads are NOT here; they
// live in `authorizeRead`.
export const ROLE_OPERATIONS: Readonly<Record<SessionRole, readonly SessionOperation[]>> = {
  owner: [
    "create-space",
    "add-member",
    "create-session",
    "set-audience",
    "attach-source",
    "join",
    "submit",
    "draft",
    "approve",
    "reject",
    "close",
    "export",
    "delete",
    "read",
  ],
  facilitator: [
    "create-session",
    "set-audience",
    "attach-source",
    "join",
    "submit",
    "draft",
    "approve",
    "reject",
    "close",
    "export",
    "read",
  ],
  participant: ["join", "submit"],
  observer: [],
};

/** True if the role holds the operation unconditionally. */
export function roleHasOperation(role: SessionRole, operation: SessionOperation): boolean {
  return ROLE_OPERATIONS[role].includes(operation);
}

/**
 * Authorize producing a mutating event. Fail-closed: a role that does not hold
 * the event's operation is refused with `sessions.membership_required` (the
 * subject lacks the membership/role for it) — the datalog's `deny if true`.
 */
export function authorizeAction(role: SessionRole, eventType: EventType): AuthzOutcome {
  return roleHasOperation(role, EVENT_OPERATION[eventType])
    ? ALLOWED
    : { ok: false, refusal: "sessions.membership_required" };
}

/**
 * Authorize a read against the audience policy. Owner and facilitator hold `read`
 * unconditionally. A participant may read the shared `session` audience, or a
 * `private` contribution only when they own it. An observer may read only the
 * `session` audience. Anything else is `sessions.audience_forbidden`; a role with
 * no read grant at all is `sessions.membership_required`.
 */
export function authorizeRead(
  role: SessionRole,
  audience: Audience,
  isContributionOwner: boolean,
): AuthzOutcome {
  if (roleHasOperation(role, "read")) return ALLOWED;
  if (role === "participant") {
    if (audience === "session") return ALLOWED;
    if (audience === "private" && isContributionOwner) return ALLOWED;
    return { ok: false, refusal: "sessions.audience_forbidden" };
  }
  if (role === "observer") {
    return audience === "session" ? ALLOWED : { ok: false, refusal: "sessions.audience_forbidden" };
  }
  return { ok: false, refusal: "sessions.membership_required" };
}
