# Sessions

- **Path:** `apps/sessions`
- **Purpose:** sourced collective sessions with human-approved outcomes.
- **Runtime:** Bun/React, Bun.serve WebSockets, PostgreSQL and Redis when required.
- **Owns:** spaces, membership, sessions, participant state, corpus references and exports.
- **Rust candidate:** Biscuit and exact-evidence/RAG verification.
- **Critical gates:** OIDC/session separation, tenant isolation, reconnect, proxy-log safety, persistence, multi-instance behavior and audience-specific exports.
