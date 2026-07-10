# rumble-lm

**Outil** : Rumble
**Rôle** : produit de learning/facilitation sourcé et sessions live
**deployment_class** : product-linkable
**Maturité** : contract-first — runtime scaffolding présent, pas un produit fini
**Place dans la chaîne DoD** : exprime le besoin de session sourcée et produit contrats/fixtures qui doivent traverser Portal, Gear, Wrench et Bolt.
**Doctrine** : source-grounded, preuve avant promesse ; le scaffolding runtime ne vaut pas maturité produit.
**Souveraineté** : licences MIT/Apache/MPL compatibles ; pas d’AGPL/SSPL dans la chaîne versionnée.

## Ce que ça fait

Cadre des sessions pédagogiques avec sources, citations, rôles et délégation bornée. Le dépôt contient des contrats, stubs et briques runtime optionnelles ; l’expérience complète, durable et e2e reste à construire.

## Où ça se branche

- Amont : specs Rumble LM et contrats partagés dans [ecosystem/specs/rumble-lm](https://github.com/constantin-jais/constantin-jais/tree/main/ecosystem/specs/rumble-lm).
- Aval attendu : [portal-forge](https://github.com/constantin-jais/portal-forge)/Portal pour l’UI, [gear-loader](https://github.com/constantin-jais/gear-loader) + [gear-memory](https://github.com/constantin-jais/gear-memory), Wrench puis Bolt handoff.
- Contrats : session source-grounded P0, delegated authorization Biscuit, futures `CitationValidation`.

[![CI](https://github.com/constantin-jais/rumble-lm/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/constantin-jais/rumble-lm/actions/workflows/ci.yml)
[![Security](https://github.com/constantin-jais/rumble-lm/actions/workflows/security.yml/badge.svg?branch=main)](https://github.com/constantin-jais/rumble-lm/actions/workflows/security.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## Dogfooding

This repository is part of the **Libre IA** tool family — one tool, one job, stacked.

Current visible evidence:

- CI and security workflows exercise source-grounded learning-session contracts;
- README maturity notes keep provider, citation, retention, and UI limits explicit;
- `live-question-grounding.v0.1` fixtures prove the public question projection carries citation status without source text or raw section ids;
- stubs validate boundaries before hosted or multi-user claims are made.

Expected next evidence:

- publish richer example session/citation outputs from a real RAG run;
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

Turn the P0 contract stub into a minimal grounded session workflow with real RAG citation outputs, retention defaults, and BYOK/provider policy. The live protocol now exposes an explicit public grounding summary; it is not yet a full production citation-validation workflow.

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
- Agentic orchestration internals: belongs to `bolt-cos-matic`.
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
