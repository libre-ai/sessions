// Sessions' adoption of the rgpd-kit DataSubjectRightsPort (design §5,
// first adopter). Sessions owns this implementation entirely inside its
// bounded context: its event log, its tombstone table, its deletion
// receipts. Nothing here reaches another product's data.
//
// Erasure semantics against the append-only floor: session_events excludes
// UPDATE/DELETE for the application role (0001_sessions.sql), so Art. 17 is
// honored the DATA-LIFECYCLE way — the accepted deletion transaction removes
// LOGICAL access (tombstone in session_deleted_subjects, checked by every
// RGPD read path) and persists the deletion receipt atomically via
// executeActiveDeletion; physical compaction of the log follows the
// owner-scoped retention path, never the application role.
//
// AUTHORIZATION PRECONDITION (port contract): the caller authorized the
// actor, tenant and scope before invoking; reaching this code means that
// gate passed. Methods still fail closed with typed refusals.

import {
  type BlobStorePort,
  executeActiveDeletion,
  type ProjectionCachePort,
  type SqlExecutor,
  withTenantDbTransaction,
} from "@libre-ai/data";
import {
  type AccessRequestResult,
  type DataCategoryDeclaration,
  type DataSubjectRightsPort,
  deriveSubjectDigest,
  type ErasureRequestResult,
  InvalidSubjectIdentifierError,
  type PortabilityRequestResult,
  type RestrictionRequestResult,
} from "@libre-ai/rgpd-kit";

export interface SessionsRgpdDeps {
  readonly executor: SqlExecutor;
  readonly cache: ProjectionCachePort;
  readonly blobs: BlobStorePort;
  /** Injected clock (ISO timestamp) so evidence is reproducible in tests. */
  readonly now: () => string;
  readonly newRequestId: () => string;
}

// Categories Sessions holds for a participant: what they said
// (communication), who did what (audit), and when (timestamp). The event log
// is append-only, so logical erasure is immediate and physical compaction is
// deferred to the retention path — hence "deferred".
const SESSIONS_CATEGORIES: readonly DataCategoryDeclaration[] = [
  {
    category: "communication",
    description: "Conversation events and structured contributions authored by the subject",
    legalBasis: "contract",
    retentionRule: "sessions-content",
    erasureScope: "deferred",
  },
  {
    category: "audit",
    description: "Which session actions the subject performed",
    legalBasis: "contract",
    retentionRule: "sessions-content",
    erasureScope: "deferred",
  },
  {
    category: "timestamp",
    description: "When the subject's session actions occurred",
    legalBasis: "contract",
    retentionRule: "sessions-content",
    erasureScope: "deferred",
  },
];

// Sentinels thrown inside the deletion transaction to abort it; mapped to
// typed refusals at the port surface, never surfaced as exceptions.
class SubjectUnknownSentinel extends Error {}
class AlreadyErasedSentinel extends Error {}

interface ActorRow {
  readonly actor_id: string;
}

async function resolveSubjectActors(
  tx: SqlExecutor,
  tenantId: string,
  subjectDigest: string,
): Promise<readonly string[]> {
  // The log stores plaintext actor ids while the port speaks digests: digest
  // every distinct human actor of the tenant and keep the matches. The
  // explicit tenant_id predicate is defense in depth over RLS (K4 finding:
  // never rely on the barrier alone), and no plaintext enters any signature.
  // TODO(rgpd-scale): O(distinct actors) sha-256 per request is fine at
  // walking-skeleton scale but unbounded; persist an indexed actor_digest at
  // insertion before any tenant can hold thousands of distinct actors.
  const actors = await tx.query<ActorRow>(
    "SELECT DISTINCT actor_id FROM session_events WHERE actor_kind = 'human' AND tenant_id = $1",
    [tenantId],
  );
  const matches: string[] = [];
  for (const row of actors.rows) {
    if ((await deriveSubjectDigest(tenantId, row.actor_id)) === subjectDigest) {
      matches.push(row.actor_id);
    }
  }
  return matches;
}

async function isTombstoned(
  tx: SqlExecutor,
  tenantId: string,
  subjectDigest: string,
): Promise<boolean> {
  // Explicit tenant_id over RLS, same defense in depth as above.
  const tombstone = await tx.query(
    "SELECT 1 FROM session_deleted_subjects WHERE tenant_id = $1 AND subject_digest = $2",
    [tenantId, subjectDigest],
  );
  return tombstone.rows.length > 0;
}

export function createSessionsDataSubjectRights(deps: SessionsRgpdDeps): DataSubjectRightsPort {
  return {
    async verifySubject(tenantId, subjectIdentifier) {
      return withTenantDbTransaction(deps.executor, tenantId, async (tx) => {
        const participant = await tx.query(
          "SELECT 1 FROM session_events WHERE actor_kind = 'human' AND actor_id = $1 AND tenant_id = $2 LIMIT 1",
          [subjectIdentifier, tenantId],
        );
        if (participant.rows.length === 0) {
          return null;
        }
        try {
          return await deriveSubjectDigest(tenantId, subjectIdentifier);
        } catch (error) {
          // An identifier the digest refuses is unverifiable, not an outage.
          if (error instanceof InvalidSubjectIdentifierError) {
            return null;
          }
          throw error;
        }
      });
    },

    async handleAccessRequest(tenantId, subjectDigest): Promise<AccessRequestResult> {
      const requestId = deps.newRequestId();
      return withTenantDbTransaction(deps.executor, tenantId, async (tx) => {
        if (await isTombstoned(tx, tenantId, subjectDigest)) {
          return { status: "refused", requestId, refusal: "sessions.rgpd.subject_erased" };
        }
        const actors = await resolveSubjectActors(tx, tenantId, subjectDigest);
        if (actors.length === 0) {
          return { status: "refused", requestId, refusal: "sessions.rgpd.subject_unknown" };
        }
        const events: unknown[] = [];
        for (const actorId of actors) {
          const rows = await tx.query(
            `SELECT session_id, sequence, event_id, revision, type, occurred_at, data, recorded_at
             FROM session_events
             WHERE actor_kind = 'human' AND actor_id = $1 AND tenant_id = $2
             ORDER BY session_id, sequence`,
            [actorId, tenantId],
          );
          for (const row of rows.rows) {
            events.push({
              sessionId: row.session_id,
              sequence: row.sequence,
              eventId: row.event_id,
              revision: row.revision,
              type: row.type,
              occurredAt: row.occurred_at,
              data: row.data,
              recordedAt: row.recorded_at,
            });
          }
        }
        return {
          status: "fulfilled",
          requestId,
          subjectDigest,
          dataExport: { schemaVersion: "libre-ai.sessions.subject-export.v1", events },
          exportedAt: deps.now(),
          categories: SESSIONS_CATEGORIES.map((declaration) => declaration.category),
        };
      });
    },

    async handleErasureRequest(tenantId, subjectDigest): Promise<ErasureRequestResult> {
      const requestId = deps.newRequestId();
      const now = deps.now();
      let recordsAffected = 0;
      try {
        const receipt = await executeActiveDeletion(deps.executor, deps.cache, deps.blobs, {
          id: requestId,
          tenantId,
          owner: "sessions",
          subjectDigests: [subjectDigest],
          // Self-service request: attribution is the opaque digest itself —
          // the caller authorized the actor before invoking the port.
          requestedBy: subjectDigest,
          requestedAt: now,
          completedAt: now,
          deleteActiveRows: async (tx) => {
            if (await isTombstoned(tx, tenantId, subjectDigest)) {
              throw new AlreadyErasedSentinel();
            }
            const actors = await resolveSubjectActors(tx, tenantId, subjectDigest);
            if (actors.length === 0) {
              throw new SubjectUnknownSentinel();
            }
            for (const actorId of actors) {
              const counted = await tx.query<{ count: string | number }>(
                "SELECT count(*) AS count FROM session_events WHERE actor_kind = 'human' AND actor_id = $1 AND tenant_id = $2",
                [actorId, tenantId],
              );
              recordsAffected += Number(counted.rows[0]?.count ?? 0);
            }
            // The tombstone IS the logical deletion: every RGPD read path
            // refuses the subject from this row on, in the same accepted
            // transaction as the receipt. The receipt's `postgresql:
            // deleted` outcome attests exactly this accepted deletion
            // (DATA-LIFECYCLE §Explicit deletion item 6: logical access
            // removed in the transaction, physical compaction may follow on
            // the retention path) — recordsAffected counts the rows made
            // inaccessible, not rows physically removed.
            await tx.query(
              `INSERT INTO session_deleted_subjects (tenant_id, subject_digest, receipt_id, deleted_at)
               VALUES ($1, $2, $3, $4)`,
              [tenantId, subjectDigest, requestId, now],
            );
          },
        });
        return {
          status: "fulfilled",
          requestId,
          subjectDigest,
          erasedAt: receipt.completedAt,
          deletionReceiptId: receipt.id,
          recordsAffected,
          categoriesErased: SESSIONS_CATEGORIES.map((declaration) => declaration.category),
        };
      } catch (error) {
        if (error instanceof AlreadyErasedSentinel) {
          return { status: "refused", requestId, refusal: "sessions.rgpd.already_erased" };
        }
        if (error instanceof SubjectUnknownSentinel) {
          return { status: "refused", requestId, refusal: "sessions.rgpd.subject_unknown" };
        }
        throw error;
      }
    },

    async handleRestrictionRequest(): Promise<RestrictionRequestResult> {
      // Deferred (README, design §6): restriction needs a flag store and a
      // read-path contract of its own; refusing typed beats pretending.
      return {
        status: "refused",
        requestId: deps.newRequestId(),
        refusal: "sessions.rgpd.not_implemented",
      };
    },

    async handlePortabilityRequest(): Promise<PortabilityRequestResult> {
      return {
        status: "refused",
        requestId: deps.newRequestId(),
        refusal: "sessions.rgpd.not_implemented",
      };
    },

    async listDataCategories(): Promise<readonly DataCategoryDeclaration[]> {
      return SESSIONS_CATEGORIES;
    },
  };
}
