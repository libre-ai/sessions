import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import type { EventType } from "../domain/session-event";
import {
  authorizeAction,
  authorizeRead,
  ROLE_OPERATIONS,
  type SessionOperation,
  type SessionRole,
} from "./session-authorization";

const ALLOWED = { ok: true } as const;
const MEMBERSHIP = { ok: false, refusal: "sessions.membership_required" } as const;
const AUDIENCE = { ok: false, refusal: "sessions.audience_forbidden" } as const;

describe("authorizeAction — role × event operation", () => {
  const ALL_EVENTS: EventType[] = [
    "member-added",
    "session-created",
    "source-attached",
    "participant-joined",
    "contribution-submitted",
    "synthesis-drafted",
    "outcome-approved",
    "outcome-rejected",
    "session-closed",
    "session-exported",
    "session-deleted",
  ];

  test("an owner may produce every event type", () => {
    for (const type of ALL_EVENTS) expect(authorizeAction("owner", type)).toEqual(ALLOWED);
  });

  test("a facilitator may not add members or delete the session", () => {
    expect(authorizeAction("facilitator", "member-added")).toEqual(MEMBERSHIP);
    expect(authorizeAction("facilitator", "session-deleted")).toEqual(MEMBERSHIP);
    expect(authorizeAction("facilitator", "session-created")).toEqual(ALLOWED);
    expect(authorizeAction("facilitator", "outcome-approved")).toEqual(ALLOWED);
  });

  test("a participant may only join and submit", () => {
    expect(authorizeAction("participant", "participant-joined")).toEqual(ALLOWED);
    expect(authorizeAction("participant", "contribution-submitted")).toEqual(ALLOWED);
    expect(authorizeAction("participant", "synthesis-drafted")).toEqual(MEMBERSHIP);
    expect(authorizeAction("participant", "outcome-approved")).toEqual(MEMBERSHIP);
  });

  test("an observer may produce no mutating event", () => {
    for (const type of ALL_EVENTS) expect(authorizeAction("observer", type)).toEqual(MEMBERSHIP);
  });
});

describe("authorizeRead — audience policy", () => {
  test.each<SessionRole>([
    "owner",
    "facilitator",
  ])("%s reads any audience unconditionally", (role) => {
    expect(authorizeRead(role, "private", false)).toEqual(ALLOWED);
    expect(authorizeRead(role, "facilitators", false)).toEqual(ALLOWED);
    expect(authorizeRead(role, "session", false)).toEqual(ALLOWED);
  });

  test("a participant reads the session audience, and a private contribution only when owner", () => {
    expect(authorizeRead("participant", "session", false)).toEqual(ALLOWED);
    expect(authorizeRead("participant", "private", true)).toEqual(ALLOWED);
    expect(authorizeRead("participant", "private", false)).toEqual(AUDIENCE);
    expect(authorizeRead("participant", "facilitators", false)).toEqual(AUDIENCE);
  });

  test("an observer reads only the session audience", () => {
    expect(authorizeRead("observer", "session", false)).toEqual(ALLOWED);
    expect(authorizeRead("observer", "private", true)).toEqual(AUDIENCE);
    expect(authorizeRead("observer", "facilitators", false)).toEqual(AUDIENCE);
  });
});

describe("conformance to the locked sessions-v1.datalog", () => {
  const policy = readFileSync(
    join(import.meta.dir, "..", "..", "..", "..", "contracts", "authz", "sessions-v1.datalog"),
    "utf8",
  );

  // Parse the UNCONDITIONAL grants: role("X") ... [op, op, ...].contains($operation).
  function unconditionalGrants(): Map<string, Set<string>> {
    const grants = new Map<string, Set<string>>();
    const rule =
      /role\(\$user,\s*"([a-z]+)"\),\s*operation\(\$operation\),\s*\[([^\]]+)\]\.contains/g;
    for (const match of policy.matchAll(rule)) {
      const role = match[1];
      const ops = (match[2] ?? "").split(",").map((o) => o.trim().replace(/^"|"$/g, ""));
      if (role !== undefined) grants.set(role, new Set(ops));
    }
    return grants;
  }

  test("ROLE_OPERATIONS matches the datalog's unconditional grants", () => {
    const grants = unconditionalGrants();
    for (const role of ["owner", "facilitator", "participant"] as const) {
      expect(new Set(ROLE_OPERATIONS[role])).toEqual(grants.get(role) as Set<SessionOperation>);
    }
    // The observer has no unconditional grant (only an audience-conditional read).
    expect(grants.has("observer")).toBe(false);
    expect(ROLE_OPERATIONS.observer).toEqual([]);
  });

  test("the policy is deny-by-default", () => {
    expect(policy).toContain("deny if true");
  });
});
