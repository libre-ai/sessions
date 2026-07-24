-- Sessions retention compaction grants (retention execution + physical
-- compaction design, owner arbitrage 2026-07-24; first adopter of the
-- libre_ai_retention role from packages/data 0003_retention_role.sql). These
-- three GRANTs are the libre_ai_retention role's ONLY grants anywhere in
-- Sessions: the role reaches this bounded context through nothing else. The
-- sweep spec that assumes them lives in src/rgpd/retention.ts; it runs every
-- deletion under the single retention barrier (withTenantRetentionTransaction).
--
-- NAMED CONTROLLER DEVIATION (owner decision 2026-07-24) — the design's §3
-- summary says the retention role gets "ONLY SELECT, DELETE ON session_events".
-- But §4 phase-2 (delta-review, Critical) REQUIRES re-checking the §5
-- restriction exclusion INSIDE the deleting transaction, which runs under
-- libre_ai_retention — and that re-check is impossible without reading the
-- evidence tables. Resolved in favor of the stronger invariant: the role also
-- gets read-only SELECT on session_restricted_subjects (the exclusion
-- predicate) and on session_deleted_subjects (the erasure carve-out — a
-- restricted-then-erased subject is compactable — and the receipt ids that
-- evidence deferred compaction, #229). No INSERT/UPDATE/DELETE on any evidence
-- table: they stay UNSWEEPABLE BY GRANT. FORCE RLS (policies carry no TO
-- clause) binds libre_ai_retention on all three tables exactly as it binds
-- libre_ai_app, so every read and the delete are tenant-scoped by the
-- transaction GUC in addition to the spec's explicit tenant_id predicates.
--
-- NEVER SWEPT — tombstones (session_deleted_subjects), audit rows
-- (session_subject_audit, not granted here at all) and restriction rows
-- (session_restricted_subjects) are the evidence; the sweep reads them and
-- deletes ONLY session_events (a whole session stream at a time, DECISION 1
-- G-A — the causal log rejects row-level holes). The exclusion the spec reads
-- lives in session_restricted_subjects (0004_restriction.sql): a subject whose
-- current state is restricted or lift-pending is never swept.

GRANT SELECT, DELETE ON session_events TO libre_ai_retention;
GRANT SELECT ON session_restricted_subjects TO libre_ai_retention;
GRANT SELECT ON session_deleted_subjects TO libre_ai_retention;
