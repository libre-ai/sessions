// The Sessions compaction spec (retention execution + physical compaction
// design, 2026-07-24; first adopter of packages/data's runRetentionSweep). One
// responsibility: supply the product-specific halves of a CompactionSpec —
// the advisory expiry selection (APP barrier), the authoritative per-session
// compaction with its in-transaction re-check (RETENTION barrier), and the
// named legal-hold deferral. All product knowledge lives here; the orchestrator
// (packages/data) owns the two-phase control flow and sees only opaque ids.
//
// DECISION 1 — granularity G-A (session-lifecycle): the unit of physical
// deletion is the WHOLE session stream. The causal log rejects row-level holes
// (loadSessionState throws SessionStreamCorruptError on a gap), so a session is
// swept only when its ENTIRE stream is past the retention window, and the
// delete is one composite-PK prefix DELETE — never a per-row or per-subject
// deletion.
//
// AGE COLUMN — recorded_at (server-set at append; occurred_at is
// client-supplied and forgeable). A session's age is max(recorded_at) of its
// stream: it is expired only when its NEWEST event is past the window.
//
// CROSS-INVARIANT (§5 of both the retention and restriction designs): a session
// containing at least one event of a currently-restricted or lift-pending
// subject is excluded from the sweep entirely (Art. 18(2): under restriction
// only storage is permitted, and deletion is processing — Art. 4(2)). The one
// carve-out: a subject who was restricted and LATER erased at their own request
// is compactable — the erasure request is the subject's Art. 18(2) consent to
// that one processing, so it supersedes the restriction. The exclusion set is
// therefore {current state restricted|lift-pending} MINUS {tombstoned}.
//
// GRANTS — this spec reads session_restricted_subjects and
// session_deleted_subjects INSIDE the retention barrier to re-check the
// exclusion and to collect receipt ids. That read is the named controller
// deviation from the design's §3 grant summary, granted by
// 0005_retention_grants.sql (SELECT only on both evidence tables; they stay
// unsweepable by grant). Every statement carries an explicit `tenant_id = $n`
// predicate — defense in depth over RLS (K4: never rely on the barrier alone).

import type { CompactionSpec, SqlExecutor } from "@libre-ai/data";

const OWNER = "sessions";
const RULE_ID = "sessions-content";

// Subjects whose CURRENT restriction state (highest entry_seq — the same
// current-state semantics as isRestricted) pauses the sweep: `restricted` or
// `lift-pending`, MINUS any subject already erased at their own request (a
// tombstone in session_deleted_subjects). Both phases share this CTE; it binds
// only $1 = tenant_id, so it composes with each query's remaining parameters.
const EXCLUDED_SUBJECTS_CTE = `excluded_subjects AS (
  SELECT cur.subject_digest
  FROM (
    SELECT DISTINCT ON (subject_digest) subject_digest, state
    FROM session_restricted_subjects
    WHERE tenant_id = $1
    ORDER BY subject_digest, entry_seq DESC
  ) cur
  WHERE cur.state IN ('restricted', 'lift-pending')
    AND NOT EXISTS (
      SELECT 1 FROM session_deleted_subjects d
      WHERE d.tenant_id = $1 AND d.subject_digest = cur.subject_digest
    )
)`;

interface ReceiptRow {
  readonly receipt_id: string;
}

interface RecheckRow {
  readonly expired: boolean | null;
  readonly excluded: boolean;
}

export const sessionsCompactionSpec: CompactionSpec = {
  owner: OWNER,
  ruleId: RULE_ID,

  /**
   * Phase 1 (APP barrier, read-only): the ids of fully-expired sessions minus
   * the restriction exclusion. Expiry is `max(recorded_at) + window <= now`
   * (the newest event is past the window), so no partially-expired stream is
   * ever selected. Ordered by session id for a deterministic sweep. ADVISORY
   * only — the guard is compactUnit's own re-check.
   */
  async selectExpiredUnits(
    tx: SqlExecutor,
    tenantId: string,
    now: string,
    retentionDays: number,
  ): Promise<readonly string[]> {
    const res = await tx.query<{ session_id: string }>(
      `WITH ${EXCLUDED_SUBJECTS_CTE}
       SELECT e.session_id
       FROM session_events e
       WHERE e.tenant_id = $1
       GROUP BY e.session_id
       HAVING max(e.recorded_at) + make_interval(days => $3) <= $2::timestamptz
          AND NOT EXISTS (
            SELECT 1 FROM session_events x
            WHERE x.tenant_id = $1 AND x.session_id = e.session_id
              AND x.actor_digest IN (SELECT subject_digest FROM excluded_subjects)
          )
       ORDER BY e.session_id`,
      [tenantId, now, retentionDays],
    );
    return res.rows.map((row) => row.session_id);
  },

  /**
   * Phase 2 (RETENTION barrier, one bounded transaction per session):
   * re-check BOTH predicates inside the deleting transaction, then compact.
   *   (a) expiry — the session still has no event newer than the window (a
   *       fresh event appended after selection ⇒ deleted:false, untouched);
   *   (b) exclusion — no event belongs to a currently-restricted or
   *       lift-pending, not-yet-erased subject (a restriction that landed
   *       after selection ⇒ deleted:false, untouched).
   * If both hold: collect the receipt ids of tombstoned subjects whose rows
   * this compaction removes (the deferred-compaction evidence, #229), then one
   * composite-PK prefix DELETE.
   */
  async compactUnit(
    tx: SqlExecutor,
    tenantId: string,
    unitId: string,
    now: string,
    retentionDays: number,
  ): Promise<{ deleted: boolean; eventsDeleted: number; compactedReceiptIds: readonly string[] }> {
    const untouched = {
      deleted: false,
      eventsDeleted: 0,
      compactedReceiptIds: [] as readonly string[],
    };

    // A single read is the authoritative guard: expiry (aggregate) AND the
    // restriction exclusion (scalar EXISTS). An empty stream (already
    // compacted) yields expired = NULL ⇒ not deletable, handled below.
    //
    // NAMED LIMIT (review Important #1, deferred, not silent): this guard
    // holds only ABSENT a concurrent append landing between this SELECT and
    // the DELETE below (or during the DELETE itself). Under v1 (PGlite,
    // effectively one connection; the owner-run CLI is the only writer; no
    // server write path is wired) that race is unreachable. Under READ
    // COMMITTED on a real multi-connection Postgres it is not closed: an
    // append committed in that window could still be deleted by the DELETE
    // below, or — if the DELETE commits first — the append could land right
    // after and be silently orphaned (a session row with no session-created
    // predecessor). G4 closure plan (either replaces this two-statement
    // re-check + DELETE): a single DELETE with the expiry/exclusion predicates
    // embedded directly in its WHERE clause PLUS an advisory lock keyed on
    // (tenant_id, session_id) shared with the append path (session-event-store
    // appendEvent), or SERIALIZABLE isolation with retry on the append
    // transaction. Until G4, the two-phase design here (advisory select, then
    // this re-check) is the mitigation, not a proof of exclusion.
    const recheck = await tx.query<RecheckRow>(
      `WITH ${EXCLUDED_SUBJECTS_CTE}
       SELECT
         (max(e.recorded_at) + make_interval(days => $4) <= $3::timestamptz) AS expired,
         EXISTS (
           SELECT 1 FROM session_events x
           WHERE x.tenant_id = $1 AND x.session_id = $2
             AND x.actor_digest IN (SELECT subject_digest FROM excluded_subjects)
         ) AS excluded
       FROM session_events e
       WHERE e.tenant_id = $1 AND e.session_id = $2`,
      [tenantId, unitId, now, retentionDays],
    );
    const row = recheck.rows[0];
    if (row === undefined || row.expired !== true || row.excluded) {
      return untouched;
    }

    // Receipt ids of tombstoned subjects whose rows are about to be compacted
    // — collected BEFORE the DELETE removes the actor digests they join on.
    const receipts = await tx.query<ReceiptRow>(
      `SELECT DISTINCT d.receipt_id
       FROM session_deleted_subjects d
       WHERE d.tenant_id = $1
         AND d.subject_digest IN (
           SELECT DISTINCT actor_digest FROM session_events
           WHERE tenant_id = $1 AND session_id = $2 AND actor_digest IS NOT NULL
         )
       ORDER BY d.receipt_id`,
      [tenantId, unitId],
    );

    const deleted = await tx.query(
      "DELETE FROM session_events WHERE tenant_id = $1 AND session_id = $2",
      [tenantId, unitId],
    );

    return {
      deleted: true,
      eventsDeleted: deleted.affectedRows ?? 0,
      compactedReceiptIds: receipts.rows.map((r) => r.receipt_id),
    };
  },

  /**
   * Legal-hold pre-check. v1: a documented constant-empty — no hold registry
   * exists yet, so the deferral is NAMED, never silent (same precedent as
   * executeActiveDeletion's K4 M-09 note). A future hold registry plugs in
   * here without reshaping the sweep; a non-empty result blocks it entirely.
   */
  async holds(_tx: SqlExecutor, _tenantId: string): Promise<readonly string[]> {
    return [];
  },
};
