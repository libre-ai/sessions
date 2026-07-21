import { describe, expect, test } from "bun:test";

import {
  type Outcome,
  reduce,
  type SessionEvent,
  type SessionState,
  validateEvent,
} from "./session-event";

function unwrap(outcome: Outcome<SessionState>): SessionState {
  if (!outcome.ok) throw new Error(`unexpected refusal: ${outcome.refusal}`);
  return outcome.value;
}

const CREATED = {
  schemaVersion: "libre-ai.session-event.v1",
  id: "urn:libre-ai:event:e-1",
  tenantId: "ten_aaaaaaaaaaaaaaaa",
  sessionId: "urn:libre-ai:session:s-alpha",
  sequence: 1,
  revision: 0,
  type: "session-created",
  actor: { kind: "human", id: "owner-alpha" },
  occurredAt: "2026-07-21T10:30:00Z",
  data: {},
} as const;

function raw(overrides: Record<string, unknown>): Record<string, unknown> {
  return { ...CREATED, ...overrides };
}

function validEvent(overrides: Record<string, unknown>): SessionEvent {
  const outcome = validateEvent(raw(overrides));
  if (!outcome.ok) throw new Error(`fixture invalid: ${outcome.refusal}`);
  return outcome.value;
}

const CURSOR = { ok: false, refusal: "sessions.cursor_invalid" } as const;

describe("validateEvent — accepts conformant events", () => {
  test("a session-created event with revision 0", () => {
    const outcome = validateEvent(CREATED);
    expect(outcome.ok).toBe(true);
    if (!outcome.ok) return;
    expect(outcome.value.type).toBe("session-created");
    expect(outcome.value.revision).toBe(0);
  });

  test("a contribution-submitted event with its required data", () => {
    const outcome = validateEvent(
      raw({
        type: "contribution-submitted",
        data: {
          resourceId: "urn:libre-ai:contribution:c-1",
          audience: "session",
          contentDigest: "a".repeat(64),
        },
      }),
    );
    expect(outcome.ok).toBe(true);
  });

  test("a synthesis-drafted event with an artifact reference", () => {
    const outcome = validateEvent(
      raw({
        type: "synthesis-drafted",
        data: {
          artifact: {
            id: "urn:libre-ai:artifact:a-1",
            digest: "b".repeat(64),
            mediaType: "text/markdown",
          },
        },
      }),
    );
    expect(outcome.ok).toBe(true);
  });

  test("an outcome-rejected event without a reasonCode (schema-optional)", () => {
    const outcome = validateEvent(raw({ type: "outcome-rejected", data: {} }));
    expect(outcome.ok).toBe(true);
  });

  test("a validated event is deep-frozen, including the nested artifact", () => {
    const outcome = validateEvent(
      raw({
        type: "outcome-approved",
        data: {
          artifact: {
            id: "urn:libre-ai:artifact:a-1",
            digest: "b".repeat(64),
            mediaType: "text/markdown",
          },
        },
      }),
    );
    expect(outcome.ok).toBe(true);
    if (!outcome.ok) return;
    expect(Object.isFrozen(outcome.value)).toBe(true);
    expect(Object.isFrozen(outcome.value.data)).toBe(true);
    expect(Object.isFrozen(outcome.value.actor)).toBe(true);
    expect(Object.isFrozen(outcome.value.data.artifact)).toBe(true);
    expect(() => {
      (outcome.value.data.artifact as { digest: string }).digest = "c".repeat(64);
    }).toThrow();
  });

  test.each([
    "ten_" + "a".repeat(16),
    "ten_" + "z9".repeat(30) + "abcd",
  ])("a tenantId at the length bounds: %s", (tenantId) => {
    expect(validateEvent(raw({ tenantId })).ok).toBe(true);
  });

  test.each([
    "2026-07-21T10:30:00Z",
    "2026-07-21T10:30:00.123456Z",
    "2026-07-21T10:30:00+02:00",
  ])("an RFC 3339 timestamp: %s", (occurredAt) => {
    expect(validateEvent(raw({ occurredAt })).ok).toBe(true);
  });
});

describe("validateEvent — fail-closed on contract violations", () => {
  test.each([
    ["unknown top-level key", { extra: 1 }],
    ["wrong schemaVersion", { schemaVersion: "libre-ai.session-event.v2" }],
    ["id not a urn", { id: "e-1" }],
    ["tenantId without ten_ prefix", { tenantId: "org-example" }],
    ["tenantId too short", { tenantId: "ten_short" }],
    ["sessionId not a urn", { sessionId: "s-alpha" }],
    ["sequence below 1", { sequence: 0 }],
    ["revision below 0", { revision: -1 }],
    ["non-integer sequence", { sequence: 1.5 }],
    ["unknown event type", { type: "session-frozen" }],
    ["timestamp without offset", { occurredAt: "2026-07-21T10:30:00" }],
  ])("refuses: %s", (_label, override) => {
    expect(validateEvent(raw(override))).toEqual(CURSOR);
  });

  test.each([
    ["actor id too short", { actor: { kind: "human", id: "ab" } }],
    ["actor unknown kind", { actor: { kind: "robot", id: "owner-alpha" } }],
    ["actor extra key", { actor: { kind: "human", id: "owner-alpha", name: "x" } }],
  ])("refuses a malformed actor: %s", (_label, override) => {
    expect(validateEvent(raw(override))).toEqual(CURSOR);
  });

  test("refuses an unknown data key", () => {
    expect(validateEvent(raw({ data: { note: "x" } }))).toEqual(CURSOR);
  });

  test("refuses contribution-submitted missing contentDigest", () => {
    expect(
      validateEvent(
        raw({
          type: "contribution-submitted",
          data: { resourceId: "urn:libre-ai:contribution:c-1", audience: "session" },
        }),
      ),
    ).toEqual(CURSOR);
  });

  test("refuses synthesis-drafted missing its artifact", () => {
    expect(validateEvent(raw({ type: "synthesis-drafted", data: {} }))).toEqual(CURSOR);
  });

  test("refuses an artifact missing mediaType", () => {
    expect(
      validateEvent(
        raw({
          type: "outcome-approved",
          data: { artifact: { id: "urn:libre-ai:artifact:a-1", digest: "b".repeat(64) } },
        }),
      ),
    ).toEqual(CURSOR);
  });
});

describe("reduce — append-only state machine", () => {
  test("opens a stream on session-created at sequence 1", () => {
    const outcome = reduce(null, validEvent({}));
    expect(outcome.ok).toBe(true);
    if (!outcome.ok) return;
    expect(outcome.value.eventCount).toBe(1);
    expect(outcome.value.latestSequence).toBe(1);
    expect(Object.isFrozen(outcome.value)).toBe(true);
  });

  test("refuses a first event that is not session-created", () => {
    expect(reduce(null, validEvent({ type: "member-added" }))).toEqual(CURSOR);
  });

  test("refuses a first session-created not at sequence 1", () => {
    expect(reduce(null, validEvent({ sequence: 2 }))).toEqual(CURSOR);
  });

  test("appends a contiguous event and advances the revision", () => {
    const first = reduce(null, validEvent({}));
    expect(first.ok).toBe(true);
    if (!first.ok) return;
    const next = reduce(
      first.value,
      validEvent({ type: "member-added", sequence: 2, revision: 1 }),
    );
    expect(next.ok).toBe(true);
    if (!next.ok) return;
    expect(next.value.eventCount).toBe(2);
    expect(next.value.latestRevision).toBe(1);
  });

  test("refuses an event from another tenant", () => {
    const first = reduce(null, validEvent({}));
    if (!first.ok) throw new Error("setup");
    const foreign = validEvent({
      tenantId: "ten_bbbbbbbbbbbbbbbb",
      sequence: 2,
      type: "member-added",
    });
    expect(reduce(first.value, foreign)).toEqual({
      ok: false,
      refusal: "sessions.tenant_mismatch",
    });
  });

  test("refuses an event for a different session", () => {
    const first = reduce(null, validEvent({}));
    if (!first.ok) throw new Error("setup");
    const other = validEvent({
      sessionId: "urn:libre-ai:session:s-beta",
      sequence: 2,
      type: "member-added",
    });
    expect(reduce(first.value, other)).toEqual(CURSOR);
  });

  test("refuses a sequence gap", () => {
    const first = reduce(null, validEvent({}));
    if (!first.ok) throw new Error("setup");
    expect(reduce(first.value, validEvent({ type: "member-added", sequence: 3 }))).toEqual(CURSOR);
  });

  test("refuses a rewound revision as revision_stale", () => {
    const first = reduce(null, validEvent({ revision: 2 }));
    if (!first.ok) throw new Error("setup");
    const stale = validEvent({ type: "member-added", sequence: 2, revision: 1 });
    expect(reduce(first.value, stale)).toEqual({ ok: false, refusal: "sessions.revision_stale" });
  });

  test("refuses a second session-created", () => {
    const first = reduce(null, validEvent({}));
    if (!first.ok) throw new Error("setup");
    expect(reduce(first.value, validEvent({ sequence: 2 }))).toEqual(CURSOR);
  });

  test("tracks closed / exported / deleted and refuses events after deletion", () => {
    const created = unwrap(reduce(null, validEvent({})));
    const closed = unwrap(
      reduce(created, validEvent({ type: "session-closed", sequence: 2, revision: 1 })),
    );
    expect(closed.closed).toBe(true);
    const exported = unwrap(
      reduce(closed, validEvent({ type: "session-exported", sequence: 3, revision: 2 })),
    );
    expect(exported.exported).toBe(true);
    const deleted = unwrap(
      reduce(exported, validEvent({ type: "session-deleted", sequence: 4, revision: 3 })),
    );
    expect(deleted.deleted).toBe(true);
    expect(reduce(deleted, validEvent({ type: "member-added", sequence: 5, revision: 4 }))).toEqual(
      CURSOR,
    );
  });
});
