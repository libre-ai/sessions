// The Sessions Art. 18 restriction state store and its two-step lift state
// machine (WP-G3-S01). One responsibility: read and append the restriction
// state of a subject in session_restricted_subjects, and record the Art. 18(3)
// lift as an auditable notice → attestation pair.
//
// Every function takes a `tx` ALREADY inside a tenant transaction
// (withTenantDbTransaction) — the same convention as the helpers in
// data-subject-rights.ts. Each statement still carries an explicit
// `tenant_id = $n` predicate: defense in depth over RLS (K4: never rely on the
// barrier alone).
//
// State model: the CURRENT state of a subject is the row with the highest
// `entry_seq` for (tenant_id, subject_digest). The table is append-only
// (GRANT excludes UPDATE/DELETE, 0004_restriction.sql), so a lift or a
// re-restriction is a NEW row. `entry_seq` — a monotonic identity, not a
// timestamp — keys recency, so the state is deterministic under a fixed
// injected clock even when several rows share `recorded_at`.

import type { SqlExecutor } from "@libre-ai/data";
import type { RestrictionGround } from "@libre-ai/rgpd-kit";

// State-machine violation surfaced as a typed error: the message carries the
// CODE only — never a subject identifier, digest or content (PII rule).
const RESTRICTION_STATE_INVALID = "sessions.rgpd.restriction_state_invalid";

export class RestrictionStateError extends Error {
  readonly code: string;
  constructor(code: string = RESTRICTION_STATE_INVALID) {
    super(code);
    this.name = "RestrictionStateError";
    this.code = code;
  }
}

type RestrictionState = "restricted" | "lift-pending" | "lifted";

interface StateRow {
  readonly entry_seq: string | number | bigint;
  readonly state: RestrictionState;
  readonly request_id: string;
}

/**
 * The current state row for a subject: the highest `entry_seq`, or null when
 * the subject has no restriction history. `entry_seq` keys recency (not
 * `recorded_at`) so the answer is deterministic under a fixed clock.
 */
async function currentStateRow(
  tx: SqlExecutor,
  tenantId: string,
  subjectDigest: string,
): Promise<StateRow | null> {
  const result = await tx.query<StateRow>(
    `SELECT entry_seq, state, request_id
     FROM session_restricted_subjects
     WHERE tenant_id = $1 AND subject_digest = $2
     ORDER BY entry_seq DESC
     LIMIT 1`,
    [tenantId, subjectDigest],
  );
  return result.rows[0] ?? null;
}

/**
 * EXPORTED read-path contract every future export/synthesis/sweep surface
 * consults: is the subject's processing currently restricted? True iff the
 * current state is `restricted` or `lift-pending` (a lift only completes once
 * `confirmLift` records the notice attestation); false when there is no
 * history or the current state is `lifted`.
 */
export async function isRestricted(
  tx: SqlExecutor,
  tenantId: string,
  subjectDigest: string,
): Promise<boolean> {
  const current = await currentStateRow(tx, tenantId, subjectDigest);
  return current !== null && (current.state === "restricted" || current.state === "lift-pending");
}

/**
 * Append the initial `restricted` state row (Art. 18(1)). The caller
 * (handleRestrictionRequest) has already checked the refusal ladder; this is
 * the write. `requestId` is the port request's id — the same id returned on
 * the fulfillment — so the state row cross-references the response.
 */
export async function restrict(
  tx: SqlExecutor,
  tenantId: string,
  subjectDigest: string,
  ground: RestrictionGround,
  requestId: string,
  now: string,
): Promise<void> {
  await tx.query(
    `INSERT INTO session_restricted_subjects
       (tenant_id, subject_digest, state, ground, request_id, recorded_at)
     VALUES ($1, $2, 'restricted', $3, $4, $5)`,
    [tenantId, subjectDigest, ground, requestId, now],
  );
}

/**
 * Art. 18(3) step 1 — begin lifting a restriction. Precondition: the current
 * state is exactly `restricted`, else a typed RestrictionStateError.
 *
 * Synthetic lift id: `lift_<entry_seq of the restricted row being lifted>`.
 * That entry_seq is known here (read for the precondition) BEFORE any INSERT,
 * which matters because `entry_seq` is GENERATED ALWAYS AS IDENTITY — a row
 * cannot reference its OWN seq in the same INSERT, and UPDATE is not granted.
 * The lift-pending row stores `request_id = liftId` and the audit row uses the
 * same id, keeping the in-progress/fulfilled audit pair joinable and
 * deterministic under a fixed clock.
 */
export async function requestLift(
  tx: SqlExecutor,
  tenantId: string,
  subjectDigest: string,
  now: string,
): Promise<void> {
  const current = await currentStateRow(tx, tenantId, subjectDigest);
  if (current === null || current.state !== "restricted") {
    throw new RestrictionStateError();
  }
  const liftId = `lift_${current.entry_seq}`;
  await tx.query(
    `INSERT INTO session_restricted_subjects
       (tenant_id, subject_digest, state, ground, request_id, recorded_at)
     VALUES ($1, $2, 'lift-pending', NULL, $3, $4)`,
    [tenantId, subjectDigest, liftId, now],
  );
  // Art. 19 notice obligation opened: pending until the owner attests it.
  await tx.query(
    `INSERT INTO session_subject_audit
       (tenant_id, request_id, subject_digest, right_type, status, detail, receipt_id, recorded_at)
     VALUES ($1, $2, $3, 'restriction', 'in-progress', 'sessions.rgpd.notice_required', NULL, $4)`,
    [tenantId, liftId, subjectDigest, now],
  );
}

/**
 * Art. 18(3) step 2 — complete the lift, callable only once the notice
 * obligation is discharged (v1 = owner attestation). Precondition: the current
 * state is exactly `lift-pending`, else the same typed error.
 *
 * The `lifted` row and the fulfilled audit row reuse the SAME `lift_<entry_seq>`
 * id the pending step created — recovered from the lift-pending row's
 * `request_id` — so the whole lift sequence stays joined under one id.
 */
export async function confirmLift(
  tx: SqlExecutor,
  tenantId: string,
  subjectDigest: string,
  now: string,
): Promise<void> {
  const current = await currentStateRow(tx, tenantId, subjectDigest);
  if (current === null || current.state !== "lift-pending") {
    throw new RestrictionStateError();
  }
  const liftId = current.request_id;
  await tx.query(
    `INSERT INTO session_restricted_subjects
       (tenant_id, subject_digest, state, ground, request_id, recorded_at)
     VALUES ($1, $2, 'lifted', NULL, $3, $4)`,
    [tenantId, subjectDigest, liftId, now],
  );
  // Notice attested: the lift is now effective.
  await tx.query(
    `INSERT INTO session_subject_audit
       (tenant_id, request_id, subject_digest, right_type, status, detail, receipt_id, recorded_at)
     VALUES ($1, $2, $3, 'restriction', 'fulfilled', 'sessions.rgpd.notice_attested', NULL, $4)`,
    [tenantId, liftId, subjectDigest, now],
  );
}
