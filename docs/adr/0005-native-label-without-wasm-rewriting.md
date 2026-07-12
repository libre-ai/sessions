# ADR-0005 — Native HTML label without WASM rewriting

- Status: Accepted
- Date: 2026-07-13
- Supersedes: the `dioxus-primitives::label::Label` implementation choice recorded by UI increment I1

## Context

The UI increment I1 adopted `dioxus-primitives` for one use only: `Label` inside `TextInput`. Its rendered accessibility contract was the native `<label for="…">` association already covered by SSR tests.

At the pinned revision, including that single component caused the crate to emit global focus-trap/Manganis assets and to retain machine-dependent source-path metadata in the WASM. Keeping the dependency while producing the owner bundle required deleting an otherwise unused asset and parsing/replacing bytes inside the generated WASM. That packaging patch was coupled to private binary details and was therefore fragile across compiler, linker and crate changes.

## Decision

`TextInput` uses Dioxus native HTML syntax to render `<label for="…">` directly. The existing accessibility assertions for the label/input association remain blocking. `dioxus-primitives`, its git-source exemption and all focus-trap-specific output handling are removed.

The owner finalizer treats WASM as opaque. It may rename the generated file from the SHA-256 of its exact bytes and update the JavaScript reference, but it must not parse or mutate WASM internals.

## Consequences

Native HTML provides the same label semantics with less supply-chain surface and no unused client runtime. Removing binary rewriting also strengthens CSP reasoning and build reproducibility: the attested WASM is exactly the compiler output, not a post-processed derivative. Content-addressed names and `SHA256SUMS` continue to bind the delivered package to its final bytes.

This decision does not invalidate the historical I1 record: that increment did adopt and test the primitive. It supersedes only the implementation choice after artifact review exposed its global output costs.

## Reconsideration

The primitive can be reconsidered if upstream offers modular features that include `Label` without globally emitting unrelated JavaScript/Manganis assets or machine-path metadata. Reintroduction would require a clean one-JS/one-WASM topology, unchanged WASM bytes and the current accessibility, CSP, supply-chain and repeatability gates to remain green.
