# Front-end & Portal Client Platform — Design Spec (SP-C)

- Status: Proposed
- Date: 2026-06-28
- Related: docs/adr/0001-product-architecture-and-boundaries.md, docs/specs/2026-06-28-collaborative-spaces-authz-design.md (SP-A auth), docs/specs/2026-06-28-signed-classification-clearance-design.md (SP-B), docs/specs/2026-06-27-presto-matic-design.md (clients, live protocol)
- Scope: the client architecture and Portal integration. Amended by the 2026-07-02 ecosystem decision: Rust-first product cores consume Portal client-platform contracts. Dioxus/PWA is the fast default path; SwiftUI/Compose native paths are first-class when product need and local verification justify them.

## Context

Presto-Matic's center of gravity is a **personal grounded notebook** (daily surface) with a **live-collaboration differentiator**. Two client surfaces, deliberately asymmetric:

- **Notebook app (owner)** — the primary, recurring, rich surface (RAG chat, corpus management, grounded studio). Targets: web/PWA + Tauri desktop (sovereign native). Mobile = PWA.
- **Guest / join client** — ephemeral, join-by-link, web-only, no install (the live-meeting attendee + invite-to-register). Never an app.

Decided: **Rust-first clients consuming Portal**. The wedge still starts with a web/PWA path for zero-friction access, sharing `presto-core` natively where correctness matters (protocol, state, reconnection). Portal owns shared tokens, accessibility conventions, i18n UI, and native/web adapters; product-specific LM UI remains local. SwiftUI/Compose are not rejected: they are premium/native paths once demand and local verification are proven.

## Client architecture

```
presto-core    (protocol, wire types, session state, reconnection — shared with server)
     ^
rumble-lm-ui   (product UI primitives; renamed from presto-ui)
     ^                ^
Portal          (shared tokens, a11y, i18n UI, web/native adapters)
     ^                ^
LM app          LM join   (notebook owner surface)       (lightweight guest/join surface)
```

- **Dependency invariant (ADR-0001):** client surfaces → `rumble-lm-ui` → `presto-core`, and product UI consumes Portal for shared client-platform primitives. The front **never** depends on `presto-server` as code — it talks to the server over the network (WS + HTTP). This keeps the one-way arrow intact.
- **Rendering targets:** Dioxus/PWA for the wedge; Tauri desktop for packaged desktop; SwiftUI/Compose via Portal when a native product need is proven.
- **Reactivity:** Dioxus signals; live session state mirrors `presto-core` (the same Rust types the server holds).
- **Backend contact:** WebSocket (the `presto-core` live protocol) + HTTP/REST (space CRUD, auth, ingestion). **Server-authoritative** (SP-A/live): the client renders the caps and state it is granted; it never computes score, timing, rights, or clearance. Optimistic UI is allowed; truth comes from the server.

## Auth & token transport (consistent with SP-A)

- **Web/PWA:** the Biscuit lives in an `HttpOnly; Secure; SameSite=Strict` cookie; requests carry `Sec-Fetch-Site` (server checks it). The wasm client never reads the token.
- **Tauri desktop:** token in the native process / OS secure store, sent via `Authorization` header.
- OIDC login (Authorization Code + PKCE) is a redirect to Keycloak; the front holds **no** secrets.
- The UI reflects the granted role/caps (SP-A) and clearance (SP-B): controls the user cannot exercise are absent, not merely disabled-after-click (the server still enforces — defense in depth, not UI trust).

## Portal + product UI (`rumble-lm-ui`)

- **Portal tokens only for theming:** colors as generated CSS/Swift/Kotlin tokens, plus spacing, radius, typography scale, elevation, motion. Defined once, themed by token swap. **No hard-coded colors.**
- **Portal accessibility conventions:** shared a11y contracts, focus behavior, i18n UI labels, and native/web adapter rules live in Portal.
- **Product components** live in `rumble-lm-ui` only when they carry LM-specific meaning: source card, session controls, citation review, participant activity, learning analytics. Generic buttons/inputs/dialog semantics should converge toward Portal.
- **Visual altitude:** distinctive and intentional, not a templated default — deliberate typography, a sovereign visual identity. (Apply the frontend-design principles at implementation.)
- `rumble-lm-ui` must not become the shared ecosystem design system; shared client-platform scope belongs to Portal.

## Delivery increments (wedge-first, aligned with SP-A/SP-B)

- **Increment 1 — personal notebook (web/PWA):** the authenticated solo surface (consumes SP-A inc-1: OIDC + solo space). Grounded RAG chat, corpus view/upload, the studio outputs. Portal tokens/base client primitives plus LM-specific components. Confidentiality badges read SP-B inc-1.
- **Increment 2 — guest/join + live UI:** `presto-join` (anonymous live join + invite-to-register), the live session UI (typed quiz, leaderboard, confusion heatmap, breakouts) over the existing live engine.
- **Increment 3 — Tauri desktop + offline:** sovereign native notebook; optional offline-local index/RAG via a Tauri Rust sidecar (the offline driver that justified all-Rust); full design-system surface + theming.

## Testing strategy

- **Component/unit:** product UI tests for `rumble-lm-ui` plus Portal contract fixtures for tokens/a11y/i18n UI.
- **E2E (durable, in-repo):** `playwright test` with `projects: [chromium, firefox, webkit]` per the workspace browser-tooling rule (durable cross-browser e2e lives in the repo, not the MCP). Cover: solo login → notebook; join-by-link guest flow; live session round.
- **Desktop:** Tauri's own harness for the desktop build (separate from Playwright web e2e).
- **Authn flows** explored ad-hoc with `claude-in-chrome` against a real Keycloak session before being frozen as e2e.

## Open items

- Dioxus version & desktop maturity for `presto-app` (validate against docs.rs at implementation; Dioxus desktop vs Tauri-shell of the wasm build).
- Offline-local RAG (Increment 3): embedded index/embeddings in the Tauri sidecar — scope and sovereignty (EU models).
- Design-system component inventory and a11y conformance target (WCAG level).
- PWA scope (installability, service worker, offline cache) for the web notebook.
- i18n/RTL from the token/typography layer.
- Whether `presto-join` is a separate crate or a feature-flagged build of `presto-app` (lean: separate crate, minimal deps for guests).
