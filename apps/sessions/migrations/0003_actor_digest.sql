-- Indexed opaque actor digest on the session event log (rgpd-kit follow-up,
-- TODO(rgpd-scale) of the first increment). The RGPD read paths resolve a
-- subject digest to its rows; without this column every request scans and
-- hashes O(distinct human actors). The digest is computed at append time by
-- the store (same domain-separated, tenant-scoped sha-256 as
-- @libre-ai/rgpd-kit deriveSubjectDigest), so the lookup becomes one indexed
-- equality. Structural floor: a human event MUST carry its digest — the
-- append path cannot silently skip it — while provider/system actors are not
-- data subjects and carry none. The column stores only the opaque digest,
-- never an alternate plaintext identifier.

ALTER TABLE session_events
  ADD COLUMN actor_digest text
    CONSTRAINT session_events_actor_digest_format CHECK (
      actor_digest IS NULL OR actor_digest ~ '^[a-f0-9]{64}$'
    );

-- Backfill pre-existing human rows with the EXACT derivation the RGPD read
-- paths use (@libre-ai/rgpd-kit deriveSubjectDigest, locked by its golden
-- vector test): sha-256 over 'libre-ai.rgpd.subject.v1:{tenant}:{actor}',
-- lowercase hex. Idempotent and a no-op on an empty log; without it, the
-- human floor below would refuse to apply on any deployed log.
UPDATE session_events
  SET actor_digest = encode(
    sha256(convert_to('libre-ai.rgpd.subject.v1:' || tenant_id || ':' || actor_id, 'UTF8')),
    'hex'
  )
  WHERE actor_kind = 'human' AND actor_digest IS NULL;

ALTER TABLE session_events
  ADD CONSTRAINT session_events_human_actor_digest CHECK (
    actor_kind <> 'human' OR actor_digest IS NOT NULL
  );

CREATE INDEX session_events_actor_digest
  ON session_events (tenant_id, actor_digest)
  WHERE actor_digest IS NOT NULL;
