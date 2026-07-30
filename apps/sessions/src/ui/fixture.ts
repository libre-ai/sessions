// A small, deterministic set of session states used to render the read-only
// cockpit in tests and local development. Per the spec's runtime boundaries the
// cockpit uses contract fixtures; it cannot join a real session or claim
// transport/orchestrator integration.

import type { SessionState } from "../domain/session-event";

const TENANT = "ten_aaaaaaaaaaaaaaaa";

export const COCKPIT_FIXTURE: readonly SessionState[] = [
  {
    tenantId: TENANT,
    sessionId: "urn:libre-ai:session:0001",
    closed: false,
    exported: false,
    deleted: false,
    latestSequence: 3,
    latestRevision: 2,
    eventCount: 3,
  },
  {
    tenantId: TENANT,
    sessionId: "urn:libre-ai:session:0002",
    closed: true,
    exported: false,
    deleted: false,
    latestSequence: 8,
    latestRevision: 7,
    eventCount: 8,
  },
  {
    tenantId: TENANT,
    sessionId: "urn:libre-ai:session:0003",
    closed: true,
    exported: true,
    deleted: false,
    latestSequence: 12,
    latestRevision: 10,
    eventCount: 12,
  },
  {
    tenantId: TENANT,
    sessionId: "urn:libre-ai:session:0004",
    closed: true,
    exported: true,
    deleted: true,
    latestSequence: 13,
    latestRevision: 11,
    eventCount: 13,
  },
];
