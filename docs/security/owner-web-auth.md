# Owner web authentication — architecture and threat model

Status: development wedge for issue #32. This is not a production-readiness claim.

## Trust boundary

1. Keycloak/OIDC proves an external subject through Authorization Code + PKCE.
2. `MembershipStore` atomically creates or returns that subject's one personal space and owner membership.
3. The server, as sole Biscuit emitter, mints the local space capability from that membership.
4. The Biscuit stays server-side. The browser receives only a 256-bit opaque session identifier; the server stores its SHA-256 digest and can revoke it immediately.
5. `/api/me` and `/api/spaces/current` project only `presto-core` DTOs. They return neither raw claims nor a token. The external subject is projected as a one-way pseudonymous actor id.

OIDC is therefore authentication, not local authorization. Keycloak never decides what a space is and the cookie is not itself a Biscuit capability.

## Protocol defenses

- Discovery is performed at startup. Redirect following is disabled; issuer metadata must match exactly; authorization, token and JWKS endpoints must remain on the configured issuer origin. Cleartext is accepted only on loopback for development.
- Keycloak uses Authorization Code only, with S256 PKCE. Random `state`, nonce and verifier are stored server-side for five minutes, keyed by a digest, bounded to 1,024 attempts and consumed before token exchange. A failed exchange cannot replay an attempt.
- ID tokens require RS256 plus a `kid`; signature, issuer, audience/`azp`, expiry, not-before when present, fresh `iat`, and nonce are fail-closed.
- JWKS are cached by `kid`. An unknown `kid` triggers one serialized refresh; a five-second cooldown bounds repeated misses and refresh failure is rejection, never fallback.
- Sessions expire after 15 minutes, are bounded to 10,000 entries and are removed on logout. Capability verification is repeated for each owner API read.
- The cookie is exactly `__Host-rumble_session=…; Path=/; Secure; HttpOnly; SameSite=Strict`, without `Domain`. No localStorage, sessionStorage or JS-readable token fallback exists.
- Every unsafe request carrying that cookie is checked globally. Both `Sec-Fetch-Site: same-origin` **and** an exact configured `Origin` are required. Fetch Metadata alone is never authorization.
- Auth responses use `Cache-Control: no-store`; redirects use `Referrer-Policy: no-referrer`; errors are typed but externally non-verbose.

The OIDC standard necessarily returns its short-lived authorization code and state to `/auth/callback` in the query string. The callback consumes them once and immediately redirects to `/app`; they are never copied into an application URL, response JSON, browser storage, trace or log. Session ids, ID/access tokens, nonces and PKCE verifiers never appear in URLs.

## Threats and residual risk

| Threat | Control | Residual |
| --- | --- | --- |
| Login CSRF / callback substitution | random one-use state + nonce + PKCE | compromise of server process memory remains fatal |
| Callback replay | consume-before-exchange + TTL | provider code still follows its own short TTL |
| Forged/rotated ID token | pinned RS256, total claims validation, bounded JWKS refresh | only Keycloak's configured signing authority is trusted |
| CSRF on logout/future writes | Strict cookie + exact Origin + Fetch Metadata | an origin XSS can still issue same-origin requests; CSP belongs to #36 |
| Cookie theft | HttpOnly/Secure, opaque value, 15-minute TTL, logout revocation | no device binding/DPoP in this increment |
| Permission drift | local Biscuit minted from membership, session TTL | ordinary reads do not synchronously recheck membership; sensitive operations must use `recheck_sensitive` when added |
| Enumeration/PII leakage | generic failures, no email extraction/logging, pseudonymous DTO | actor id remains a stable pseudonym |
| Resource exhaustion | bounded login/session maps, HTTP timeouts/body limits, JWKS cooldown | bounds are per process |

## Durability and multi-instance limit

Owner login transactions, opaque sessions, personal spaces and memberships are currently process-local. Restart logs every owner out. Two instances do not share login state, sessions, revocation or personal-space bootstrap and could create different spaces for the same subject. Do not enable multi-instance owner auth yet, even though existing live sessions already have Postgres/Redis adapters.

The reversible next adapter is a transactional Postgres implementation with a unique personal-space constraint per subject plus a shared expiring session/login store (and shared revocation). The browser contract and Biscuit authority boundary do not change.

## Logging rule

Do not log or instrument authorization codes, state, nonce, PKCE verifier, cookies, ID/access/Biscuit tokens, email, display name or raw OIDC subject. Operational messages may report only configuration mode and generic outcome/counts.
