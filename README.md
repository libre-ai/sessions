# Rumble LM

[![CI](https://github.com/constantin-jais/rumble-lm/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/constantin-jais/rumble-lm/actions/workflows/ci.yml)
[![Security](https://github.com/constantin-jais/rumble-lm/actions/workflows/security.yml/badge.svg?branch=main)](https://github.com/constantin-jais/rumble-lm/actions/workflows/security.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**Layer:** Rumble — Product  
**Role:** sovereign learning and facilitation platform  
**Mission:** help groups learn, discuss, and decide from source-grounded AI content in trustworthy interactive sessions.

---

## Stack role

- **Layer:** Rumble — Product.
- **Role:** sovereign learning and facilitation platform.
- **Mission:** help groups learn, discuss, and decide from source-grounded AI content in trustworthy interactive sessions.
- **Maturity:** `contract-first`.
- **Scale-ready:** no — contracts/stubs validate boundaries before a production runtime or UI.
- **Current increment:** P0 source-grounded contract stub.
- **Learning value:** pedagogy, citations, live sessions, grounding, aggregate analytics, and bounded delegation.
- **Next quality step:** define `CitationValidation`, retention defaults, and deployment-specific provider/BYOK policy.

See the ecosystem cockpit in [`constantin-jais/ecosystem/status.md`](https://github.com/constantin-jais/constantin-jais/blob/main/ecosystem/status.md).

## Dogfooding

This repository is part of the forge dogfooding loop: the ecosystem should use its own tools to make specs, maturity, contracts, releases, and product documentation observable.

Current visible evidence:

- CI and security workflows exercise source-grounded learning-session contracts;
- README maturity notes keep provider, citation, retention, and UI limits explicit;
- stubs validate boundaries before hosted or multi-user claims are made.

Expected next evidence:

- publish example session, citation, and grounding outputs;
- make retention and provider-policy checks visible as fixtures.

Dogfooding claims should stay backed by visible commands, fixtures, CI workflows, generated reports, or linked docs.

## Contributing

See:

- [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines;
- [ROADMAP.md](ROADMAP.md) for current contribution priorities;
- [issue templates](.github/ISSUE_TEMPLATE/) for bugs, docs issues, fixture/example requests, and design discussions.

## Usable today

The Rust core and server expose contract/stub proofs for source-grounded learning sessions. They are useful to validate boundaries, privacy defaults, delegation limits, and citation workflow assumptions.

## Not scale-ready yet

There is no full product runtime, UI, durable storage, provider policy instantiation, or production citation validation yet.

## Next product milestone

Turn the P0 contract stub into a minimal grounded session workflow with explicit `CitationValidation`, retention defaults, and BYOK/provider policy.

## Purpose

`rumble-lm` combines grounded knowledge work with live group engagement: documents become study material, quizzes, prompts, activities, summaries, and facilitated sessions.

The product outcome is not “chat with an LLM”; it is better learning and better collective understanding.

## Owns

- Learning/facilitation session UX for learners, facilitators, and participants.
- Source-grounded study content, activities, quizzes, and live interactions.
- Group engagement mechanics: participation, feedback, timing, scoring when relevant.
- Sovereign/BYO-key product experience and RGPD-aware operation.

## Does Not Own

- Generic model hosting or provider abstraction as infrastructure.
- Agentic orchestration internals: belongs to `cos-matic`.
- Raw ingestion/extraction: belongs to `gear-loader`.
- Cross-platform client primitives, tokens, accessibility, and native/web adapters: belong to Portal.
- Memory/storage/distribution primitives: belongs to Gear.
- A generic chatbot interface disconnected from learning outcomes.

## Allowed Dependencies

- Uses Bolt for orchestration when sessions need planning, generation, or agentic facilitation.
- Uses Gear Loader for document ingestion and source extraction.
- Uses Wrench for validation/inspection evidence.
- Uses Portal for client-platform primitives, tokens, accessibility, and native/web adapters.
- Uses Gear for memory, artifact integrity, provenance, and reproducible deployment paths.

## Product Vision Challenge

`rumble-lm` must be judged by learning outcomes, groundedness, session reliability, and group engagement — not by model novelty.

## P0 Contract Stub

The Rust core contains a contract-only P0 module: `presto_core::p0_contract`.

It validates the source-grounded session boundary before UI/runtime work:

- Rumble owns session workflow and citation review.
- Wrench/Gear-shaped source refs and provenance are required.
- Bolt-shaped generation is draft-only and cannot publish.
- Participant-facing exports exclude private responses by default.
- Delegations are scoped, expiring, revocable, and least-privilege.
- Default analytics are aggregate-only; no hidden learner profile.

This module is deliberately pure and stub-shaped. It must not become durable ingestion, memory, orchestration, artifact storage, or authorization infrastructure.

The server exposes two contract/stub endpoints:

```text
GET  /p0/contract/proof
POST /p0/stub/run
```

`GET /p0/contract/proof` validates the core contract.  
`POST /p0/stub/run` runs the deterministic vertical stub steps: create session, attach source refs, generate draft, validate citations, collect aggregate responses, export participant artifact, and prove delegation bounds.

Both endpoints are fixture-only: they report that no UI, Wrench, Gear, Bolt, Biscuit runtime, durable storage, or LLM provider was called.
