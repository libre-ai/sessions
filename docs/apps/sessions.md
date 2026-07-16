# Sessions

- **Path:** `apps/sessions`
- **Owner:** Experiences / Sessions
- **Runtime:** Bun.serve, React 19, WebSockets, PostgreSQL/RLS; Redis for ephemeral presence/fan-out only
- **Tenant model:** organization

## Purpose and actors

Sessions supports sourced collective work where owners/facilitators control membership and sources, participants contribute under explicit audience rules, and a human approves every shared outcome. Actors are organization owner, facilitator, participant, observer and bounded generation provider.

## Journeys

1. **Prepare:** facilitator creates session, audience policy and source set, invites participants and previews retention/export behavior.
2. **Participate/reconnect:** participant joins with scoped membership, contributes, disconnects and resumes from acknowledged event cursor without duplicate mutation.
3. **Draft/approve:** facilitator requests a bounded synthesis from allowed sources/contributions; output remains draft until attributable human approval.
4. **Export/delete:** authorized actor exports an audience-specific bundle; owner closes or deletes the session according to retention contract.

## Non-goals

- general chat, LMS or unrestricted collaborative drive ;
- exporting private responses into shared outcomes by default ;
- treating generated text as approved ;
- global participant identity/profile or cross-tenant discovery ;
- Redis/presence as authoritative history.

## Domain protocol

**Commands:** `CreateSpace`, `AddMember`, `CreateSession`, `SetAudiencePolicy`, `AttachSource`, `JoinSession`, `SubmitContribution`, `AcknowledgeEvents`, `RequestSynthesis`, `ApproveOutcome`, `RejectOutcome`, `CloseSession`, `ExportSession`, `DeleteSession`.

**Queries:** `GetSession`, `ListVisibleSources`, `GetEventsSince`, `GetParticipantState`, `GetDraftOutcome`, `PreviewAudienceExport`, `GetApprovedOutcome`.

**Events:** `MemberAdded`, `SessionCreated`, `SourceAttached`, `ParticipantJoined`, `ContributionSubmitted`, `SynthesisDrafted`, `OutcomeApproved`, `OutcomeRejected`, `SessionClosed`, `SessionExported`, `SessionDeleted`.

Authoritative session stream is append-only by tenant/session revision. WebSocket frames carry event cursor and command IDs. Presence is ephemeral and cannot authorize or prove participation.

## Refusal matrix

| Code | Refusal |
| --- | --- |
| `sessions.membership_required` | subject lacks active membership/role |
| `sessions.tenant_mismatch` | token, membership and session tenant differ |
| `sessions.audience_forbidden` | actor requests content outside audience policy |
| `sessions.private_export_forbidden` | export would include private contribution by default |
| `sessions.source_unapproved` | synthesis references a source not attached/validated |
| `sessions.outcome_unapproved` | client requests publication/export as approved |
| `sessions.cursor_invalid` | reconnect cursor is unknown or ahead of stream |
| `sessions.revision_stale` | command targets stale session revision |
| `sessions.provider_unavailable` | generation dependency unavailable/budget exceeded |

Provider failure leaves an attributable failed draft request and allows manual outcome authoring.

## Data

PostgreSQL owns organizations, spaces, memberships, sessions, source references, contribution metadata/content, append-only events, drafts, approvals and export manifests. Redis owns expiring presence, WebSocket fan-out and idempotent short leases only. Participant presence and session content/outcomes follow ADR-0002 section 3 retention. Audience classification is stored with each contribution. Historical Sessions tables are not imported; migration source is reviewed contract fixtures and explicitly accepted public/session templates.

## Authentication and authorization

OIDC subject maps to opaque browser session. Every internal token contains user, organization tenant, `role(user, role)`, root token ID and an expiration check. Resources include `space/<id>`, `session/<id>`, `contribution/<id>` and `outcome/<id>`; operations are explicit. Facilitator rights do not imply organization ownership. Generation calls receive an attenuated Biscuit limited to one session, permitted source/contribution IDs and `draft`; provider adapters never receive browser session cookies. Revocation and RLS are mandatory.

## Runtime boundaries

Bun owns HTTP/WebSocket protocol, event persistence, reconnect, audience projection and provider adapter. Rust owns Biscuit verification/authorizer policy and MAY own source/evidence validation through shared context/proof crates. RAG/vector retrieval is not authority and cannot bypass attached-source IDs. Redis adapters remain behind ephemeral interfaces.

## Accessibility and degraded mode

The complete session flow has an HTTP/keyboard fallback; WebSocket is enhancement, not the only mutation path. New events use polite announcements without stealing focus. Reconnect states, draft/approved distinction and private/shared audience are textual. Redis outage falls back to database polling and disables presence; provider outage permits manual sourced outcomes; database loss fails closed.

## Contracts

- Session Event v1 — `contracts/schemas/session-event.v1.schema.json` ;
- Session Export v1 — `contracts/schemas/session-export.v1.schema.json` ;
- Evidence Report v1 references — `contracts/schemas/evidence-report.v1.schema.json` ;
- Sessions API/WebSocket — `contracts/openapi/sessions.v1.yaml` ;
- Biscuit policy contract — `contracts/authz/sessions-v1.datalog`.

## Evidence

Unit tests cover state transitions, audience projections, cursors and idempotency. Integration uses PostgreSQL RLS and two Bun instances with Redis restart. Contract tests include private-export and cross-tenant fixtures. E2E covers owner/facilitator/participant, disconnect/reconnect, manual degraded outcome and export. Security tests inspect proxy logs for cookies/tokens/content and exercise revocation.

## Work packages

1. event/export/audience/authz contracts — Canonical Core + Specialized Rust ;
2. organization/session persistence and RLS — Web Platform ;
3. WebSocket/reconnect/ephemeral adapters — Web Platform ;
4. participant/facilitator experiences — Experiences ;
5. generation/evidence adapter — Experiences + Specialized Rust ;
6. multi-instance/privacy/accessibility qualification — Infrastructure and Release.

UI and persistence can proceed in parallel only after event/audience semantics are accepted.

## Release and rollback

Release requires two-instance reconnect, RLS/cross-tenant denial, private-export proof, human approval and manual provider-degraded journey. Migrations are backward-readable for one release. Rollback disables new commands, drains WebSocket clients and restores compatible server/UI; append-only events are never rewritten and accepted deletion remains irreversible.
