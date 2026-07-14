# Product readiness cockpit

Observation date: 2026-07-14
Last verified: 36e7efb

This is the canonical maturity cockpit for the repository.

## Legend

- proven local+CI — exercised locally and in CI
- implemented-unhosted — implemented and tested, but no hosted proof yet
- partial — present, but one required gate is still missing
- blocked — external environment or proof is missing
- later — explicitly deferred

## Maturity vs issue count

Open issue count is **1** (`#109`), but backlog size is not the maturity metric.

Current maturity:
- **contract-first**
- **executable local MVP evidence / not hosted**

Current operating constraints:
- owner auth, corpus, and membership are process-local;
- the owner path requires a mono-instance deployment;
- the participant token still travels in the WebSocket query string;
- no deployment has been made.

Recent local verification:
- Rust: **283 passed / 18 ignored**
- Playwright: **41 passed / 3 skipped**

Not yet proven in a real environment:
- real Keycloak;
- Clever HTTPS/WSS;
- proxy logs;
- physical phone;
- live DB/Redis;
- load;
- production.

## Owner / OIDC

| Capability | Status | Implementation | Evidence | Real-environment proof | Remaining gate |
| --- | --- | --- | --- | --- | --- |
| OIDC protocol checks | proven local+CI | in-process OIDC tests cover discovery, auth-code exchange, JWKS, claims validation, and bootstrap | `security/owner-web-auth.md`; Rust suite | real Keycloak on Clever not proven | staging #109 with hosted Keycloak + proxy-log proof |
| Owner session, corpus, membership | implemented-unhosted | process-local owner auth/corpus/membership; mono-instance only | `security/owner-web-auth.md`; `security/owner-corpus.md` | live DB/Redis and persistence not proven | persistence adapter + multi-instance revocation fanout |

## Corpus / RAG

| Capability | Status | Implementation | Evidence | Real-environment proof | Remaining gate |
| --- | --- | --- | --- | --- | --- |
| Retrieve → generate → verify → approve + citations | proven local+CI | bounded process-local corpus; exact-evidence gate; cited outputs | `security/owner-corpus.md`; `security/rag-exact-evidence-gate.md`; Rust tests | hosted provider / live corpus not proven | persistent storage + multi-instance corpus |
| Corpus tenancy | blocked | corpus remains process-local; mono-instance required | `security/owner-corpus.md` | multi-instance isolation not proven | `space_id` / persistent storage promotion |

## Live session participant

| Capability | Status | Implementation | Evidence | Real-environment proof | Remaining gate |
| --- | --- | --- | --- | --- | --- |
| Create / join / answer / reveal / leaderboard / late join / reconnect | proven local+CI | owner+guest Dioxus/WASM surface and live-session runtime | `docs/status/2026-06-29-session-handoff.md`; local Rust + Playwright | physical phone and hosted WSS not proven | staged device smoke on Clever |
| Participant transport | partial | participant token still rides in the WebSocket query string | `security/live-join-links.md` | proxy-log proof not proven | log-redaction proof in staging |

## PWA / mobile

| Capability | Status | Implementation | Evidence | Real-environment proof | Remaining gate |
| --- | --- | --- | --- | --- | --- |
| Shell-only PWA | proven local+CI | shell-only service worker; no API/cache persistence; reproducible bundles | `pwa-testing.md`; CI bundle gate | physical phone not proven | mobile HTTPS smoke on staging |
| Reproducible bundles | proven local+CI | canonical owner/join bundles are built and verified from the checkout | CI bundle job | hosted deployment not proven | keep bundle verification green |

## Security / operations

| Capability | Status | Implementation | Evidence | Real-environment proof | Remaining gate |
| --- | --- | --- | --- | --- | --- |
| CI and supply-chain gates | proven local+CI | fmt/check/clippy/test, cargo-deny, cargo-audit, guardrails | `.github/workflows/ci.yml`; `.github/workflows/security.yml` | none beyond local/CI | keep green on promotion branches |
| Deployment topology | implemented-unhosted | owner/corpus/membership are process-local; owner path is mono-instance | `docs/deploy/clever-cloud.md`; `security/owner-web-auth.md` | no deployment yet | staged host proof and persistence/multi-instance adapter |

## Promotion gates

- **P0** — executable local MVP evidence: already met locally/CI; keep the evidence green.
- **P1** — hosted staging #109 using topology A: real Keycloak, Clever HTTPS/WSS, proxy-log policy proof, physical-phone smoke, exactly one instance, and no `DATABASE_URL` or `REDIS_URL`.
- **P2** — persistence / multi-instance pilot: select the supported topology, then prove shared persistence, revocation fanout, restart behavior, and multi-instance behavior.

## Release stages

| Capability | Status | Implementation | Evidence | Real-environment proof | Remaining gate |
| --- | --- | --- | --- | --- | --- |
| P0 — executable local MVP evidence | proven local+CI | owner+guest Dioxus/WASM; create/join/answer/reveal/leaderboard/late join/reconnect; in-process OIDC protocol tests; bounded process-local corpus; retrieve → generate → verify → approve + citations; shell-only PWA; reproducible bundles; CI/security green | recent local verification; docs above | not hosted | none; already met |
| P1 — hosted staging #109 | blocked | topology A: real Keycloak; Clever HTTPS/WSS; proxy-log policy proof; physical phone; exactly one instance; no `DATABASE_URL` or `REDIS_URL` | `docs/deploy/clever-cloud.md`; staging cockpit | not proven | close #109 without claiming persistence or restart recovery |
| P2 — persistence / multi-instance pilot | later | supported topology selection; shared persistence; revocation fanout; restart and multi-instance behavior | none yet | not proven | finish the persistence adapter and prove the pilot |

This cockpit tracks evidence, not issue count. It is the canonical source for maturity.
