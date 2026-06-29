# ADR-0003 — Companion repositories for sovereign adjacent tooling

- Status: Accepted
- Date: 2026-06-29
- Amends: docs/adr/0001-product-architecture-and-boundaries.md
- Related: docs/adr/0002-mobile-first-webview-rust-core.md, docs/specs/2026-06-27-presto-matic-design.md

## Context

ADR-0001 rejected a premature multi-repo split for the **core Presto-Matic product**: crates remain the product's compile/API boundaries, and the product dependency arrow stays inside the Cargo workspace.

Five adjacent capabilities now have a different governance shape: they are reusable infrastructure/tooling products, each backed by an active upstream project or extracted doctrine and useful beyond Presto-Matic. Keeping them inside the Presto-Matic repo would either pull heavy/native dependencies into the product, mix dev tooling with runtime code, or obscure their own release cadence.

## Decision

Create public companion repositories under `constantin-jais/`, each with a narrow mandate and an upstream-first policy:

| Companion repo | Upstream inspiration / dependency | Role | Relationship to Presto-Matic |
| --- | --- | --- | --- |
| [`memory-card`](https://github.com/constantin-jais/memory-card) | basemind | Local agentic context: code map, repo memory, document/search layer for agents | Dev/operator tool only; never a product runtime dependency |
| [`disc-loader`](https://github.com/constantin-jais/disc-loader) | Xberg | Rich document ingestion: PDF/Office/OCR/HTML/archives into canonical text + metadata | External ingestion worker/service; integrates by queue/HTTP/object-store contract |
| [`vault-inspector`](https://github.com/constantin-jais/vault-inspector) | Scythe | SQL audit, schema linting, Postgres/pgvector/RLS/security inspection | CI/security tool consuming SQL/schema artifacts; does not replace `sqlx` |
| [`supply-depot`](https://github.com/constantin-jais/supply-depot) | Starmetal | Sovereign registry proxy/cache + supply-chain policy POC | Infrastructure POC; not on Presto-Matic's critical production path until promoted |
| [`link-cable`](https://github.com/constantin-jais/link-cable) | Agent-O-Matic distribution doctrine | Rust-first multi-platform distribution substrate: release manifests, artifact plans, checksums/signatures/provenance, sovereign install floors | External distribution tool; Agent-O-Matic is first consumer, Presto-Matic may later consume release plans/artifacts |

Presto-Matic remains the product repo. Companion repos may consume Presto-Matic contracts/artifacts, and Presto-Matic may call their services over stable interfaces, but the product must not gain accidental code dependencies on their internals.

## Boundary rules

1. **No permanent fork by default.** Each companion tracks upstream releases/tags/commits. Fork only for a blocking security/build/sovereignty patch, open the upstream PR, and remove the fork once merged.
2. **Stable contracts over code imports.** Cross-repo integration uses HTTP, queue messages, object-store keys, CLI reports, or JSON artifacts. Avoid importing companion internals into Presto-Matic.
3. **Dependency blast radius stays local.** Heavy/native parser or registry dependencies live in the companion that needs them, not in `presto-server` or the front.
4. **Sovereignty gates apply everywhere.** MIT/Apache/MPL-family licensing, no US hyperscaler requirement, EU-resident deployment defaults, no secrets in repos.
5. **Promotion requires proof.** A companion can become production-critical only after documented SLOs, rollback path, license/advisory scan, and integration tests.

## Initial integration posture

- `disc-loader`: first production candidate. It owns Xberg-specific extraction and sends canonical extracted text/metadata back to Presto-Matic for classification, integrity tagging, embedding, and retrieval.
- `vault-inspector`: CI/security companion. Start with SQL extraction/audit reports, then add live Postgres inspect when a disposable DB is available.
- `memory-card`: agent/operator acceleration. It can index Presto-Matic and Agent-O-Matic, but it must not become a hidden product requirement.
- `supply-depot`: lab/infra track. Use it to evaluate registry caching and policy enforcement; keep existing public registries as fallback until the POC is proven.
- `link-cable`: distribution substrate track. Keep publish/promote mutating commands dry-run or explicitly gated until signatures, SBOM, SLSA provenance, and `compensate` runbooks are proven.

## Consequences

- The product repo avoids parser/registry/tooling dependency bloat while still benefiting from strong upstream communities.
- Each companion can publish, iterate, and fail independently.
- Multi-repo ceremony increases; cross-repo contracts and release notes become mandatory.
- ADR-0001's monorepo guidance remains valid for **core product bricks**. This ADR carves out adjacent tooling/infrastructure whose governance boundary is already real.
