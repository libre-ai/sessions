# session-identity.v0.1 Contract

Biscuit-shaped token structure for session-workspace join-link authorization.
Aligns to `workspace-identity.v0.1` (ADR 0028, Accepted 2026-07-04).

## Token structure

```text
token = Biscuit-shaped facts:
  organization(TenantId)                      # mandatory tenant boundary
  actor(ActorId, ActorType, ActorName?)        # from workspace-identity ActorReference
  workspace(WorkspaceId)
  session(SessionId)
  role("host" | "participant")                # LM product role
  permission("read" | "comment" | "write" | "approve" | "invite" | "administer" | "delegate")
  capability("host_minting" | "answer_submit")
  expiry(Timestamp)
```

The LM runtime now mints session join tokens with these tenant/workspace/session facts for the open wedge. Cross-service/cos-matic real Biscuit verification remains owned by the authorization increment; this contract fixes the shape and fixtures so consumers can map LM session roles onto the shared `WorkspaceIdentity` facts without inventing a second permission vocabulary.

## Mapping: Host/Participant → RoleAssignment

- **Host role:**
  - `workspace_id`: from `workspace(WorkspaceId)`
  - `actor_ref`: `ActorReference { actor_id, actor_type: human, display_name?, source? }`
  - `role`: `"host"`
  - `permissions`: `[read, comment, write, approve, invite, administer]`
  - `created_at`: token mint timestamp
  - Rationale: Host may approve/administer, so it is human-only under `workspace-identity.v0.1` invariants.

- **Participant role:**
  - `workspace_id`: from `workspace(WorkspaceId)`
  - `actor_ref`: `ActorReference { actor_id, actor_type: human | agent, display_name?, source? }`
  - `role`: `"participant"`
  - `permissions`: `[read, comment, write]`
  - `created_at`: token mint timestamp
  - Rationale: `write` means submit answers/interactions; participants do not receive `approve`, `administer`, or `delegate`.

## WorkspaceIdentity root

Every fixture, HTTP session response, or token-derived fact set carries:

- `tenant_id` from `organization(TenantId)`;
- `workspace_id` from `workspace(WorkspaceId)`;
- memberships for the actors in scope;
- role assignments using the closed permission vocabulary.

For the current open `/sessions` wedge, `tenant_id` is the explicit interim boundary `tenant_local` and `workspace_id` is deterministically derived as `workspace_{session_id}`. This is a runtime bridge, not a general identity service.

No secret, bearer token, private key, prompt payload, or learner response is stored in this contract fixture.

## Cross-repo fixture

See `session-identity.v0.1.fixtures.json` for deterministic mock tokens and role facts. Canvas and ai-practices may consume those fixtures for integration tests without a live LM service or a real Biscuit verifier.
