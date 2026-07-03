# session-identity.v0.1 Contract

Biscuit token structure for session-workspace join-link authorization.
Aligns to workspace-identity.v0.1 (ADR 0028, Accepted 2026-07-03).

## Token structure

```
token = Biscuit(
  facts: [
    actor(ActorId, ActorType, ActorName),  // From workspace-identity contract
    workspace(WorkspaceId),
    session(SessionId),
    role("host" | "participant"),          // Product role (Host/Participant)
    permission("read" | "comment" | "write" | "approve" | "invite" | "administer" | "delegate"),  // Closed vocabulary, ADR 0028 amend#1
    capability("host_minting" | "answer_submit"),  // Capability gating
    expiry(Timestamp),                      // +2h from mint
  ],
  authority_keypair: SHARED_RUMBLE_WORKSPACE_KEY  // Shared across lm, canvas, ai-practices
)
```

## Mapping: Host/Participant → RoleAssignment

- **Host role:**
  - workspace_id: from token.facts.workspace(WorkspaceId)
  - actor_ref: ActorReference(actor_id, ActorType::Human | ActorType::Service, actor_name)
  - role: "host"
  - permissions: [read, comment, write, approve, invite, administer]
  - created_at: token.minted_at

- **Participant role:**
  - workspace_id: from token.facts.workspace(WorkspaceId)
  - actor_ref: ActorReference(actor_id, ActorType::Human | ActorType::Agent, actor_name)
  - role: "participant"
  - permissions: [read, comment, write]
  - created_at: token.minted_at

## Cross-repo fixture

See `session-identity.v0.1.fixtures.json` for deterministic mock tokens (canvas, ai-practices consume for integration tests without live lm).
