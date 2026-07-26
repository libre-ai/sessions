# ADR-0006 — Vendor `gear-loader` and `gear-memory` after their source repository disappeared

- Status: Accepted
- Date: 2026-07-26
- Supersedes: the `git`+`rev` dependency on `libre-ai/context-kit` recorded in `deny.toml`'s `allow-git` exemption

## Context

`Cargo.toml` took two crates from a git repository:

```toml
gear-loader = { git = "https://github.com/libre-ai/context-kit", rev = "f0c10bf3…" }
gear-memory = { git = "https://github.com/libre-ai/context-kit", rev = "f0c10bf3…", default-features = false }
```

That repository no longer exists. It answers 404 anonymously and authenticated, it is absent from the organisation listing (which does enumerate private repositories), it is absent from crates.io, and it is absent from the canonical monorepo. No other repository in the ecosystem depends on it.

The consequence was not cosmetic. **The workspace could not be built from a clean checkout on any machine.** The suite passed only where a `~/.cargo/git` cache had been populated while the repository still existed. A new machine, a fresh clone, or a CI runner could not repopulate it, because there was nothing left to fetch.

This is also why no compilation gate could be made required here, unlike in neighbouring repositories: any check that resolves the dependency graph fails on a clean runner. A repository whose only proof of correctness is one warm cache on one laptop has no proof of correctness.

The code survived in a git bundle kept outside this repository. Before anything was copied, the bundle was verified rather than trusted: its `SHA256SUMS` was checked, it was cloned for real, and the clone was confirmed to carry 59 commits, 72 files, a clean `git fsck`, and a `HEAD` equal to the exact `rev` this workspace pinned. A `git bundle verify` alone was not accepted as evidence — an earlier bundle of the same code passed that check while restoring zero files.

## Decision

The two crates are vendored into this workspace as ordinary members, `crates/gear-loader` and `crates/gear-memory`, and the git dependency is removed.

**Location.** They go under `crates/` with the six existing members, not under a `vendor/` directory. `vendor/` is Cargo's term of art for `cargo vendor` output: verbatim, machine-regenerated, resynchronised from an upstream through source replacement. None of that is true here. The upstream is gone, the code is pruned, and this workspace is now the only place it is maintained. Naming the directory `vendor/` would promise a resync path that does not exist. Placing them under `crates/` also puts them where `cargo fmt --all`, `cargo clippy --workspace`, `cargo test --workspace` and the `crates/**` REUSE annotation already reach — which was the point of vendoring rather than republishing.

**Scope.** Only what this workspace calls is kept. The upstream tree held 72 files; 5 are vendored.

| Kept                                | Why                                                             |
| ----------------------------------- | --------------------------------------------------------------- |
| `gear-loader/src/lib.rs`            | Holds all 13 `gear_loader::` symbols `presto-server` references |
| `gear-memory/src/lib.rs`            | Holds 6 of the 8 `gear_memory::` symbols referenced             |
| `gear-memory/src/ingestion.rs`      | Holds the other 2 (`SourceRefBuilder`, `ingest_source_ref`)     |
| `gear-loader/fixtures/minimal.pdf`  | Read by the PDF fail-closed test kept below                     |
| `gear-loader/fixtures/minimal.docx` | Read by the Office fail-closed test kept below                  |

| Dropped                                                                                           | Why                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `loader/src/worker.rs`, `loader/src/quarantine_store.rs`                                          | Leaf modules. `lib.rs` declares them with `pub mod` and never references them; nothing here calls them. They were the only users of `libc`, `tempfile` and `async-trait` in the crate, and `worker.rs` carried the crate's only `unsafe` FFI.         |
| `memory/src/sqlite.rs` and the `sqlite` feature                                                   | See below — this one is not merely unused.                                                                                                                                                                                                            |
| `loader/src/main.rs`, `memory/src/main.rs`                                                        | CLI binaries. This workspace links the libraries; it never invokes the executables.                                                                                                                                                                   |
| `memory/bench/storage-bench/`                                                                     | A third crate, a benchmark harness for a store this workspace does not use.                                                                                                                                                                           |
| `loader/tests/`, `memory/tests/` and their fixtures                                               | Would have added `jsonschema 0.18`, `tokio`, `syn` and `proc-macro2` as dev-dependencies to gate code paths nothing here calls. The inline `mod tests` blocks inside the vendored files are kept and do run — they are 33 of the tests counted below. |
| Per-crate `README`, `ROADMAP`, `SECURITY`, `CONTRIBUTING`, `docs/adr/`, `deny.toml`, `.gitignore` | Governance surfaces of a repository that no longer exists. This repository has its own, and two competing `SECURITY.md` files would be a worse defect than a missing one.                                                                             |

**Dropping the `sqlite` feature is a decision, not an omission.** `presto-server` already declared `default-features = false` to keep `rusqlite` — which bundles and compiles a C SQLite — out of the graph. But that suppression only worked while `gear-memory` was an external dependency. As a workspace member it is built by `cargo clippy --all-features` and `cargo test --all-features`, which would switch the feature back on and pull a C toolchain into a required gate for code no consumer calls. Removing the feature makes the existing intent structural instead of contingent.

**Licence.** The code stays under `MIT`, the licence it was published with, with its own copyright line preserved.

`loader/Cargo.toml` and `memory/Cargo.toml` both declared `license = "MIT"` at the pinned rev, and both crates carried an identical `LICENSE` file reading `Copyright (c) 2026 Constantin Jais`. There are no SPDX headers anywhere in the upstream tree, so those two declarations are the whole of the evidence, and they agree.

This is first-party code by authorship, and the canonical policy would place first-party software under `EUPL-1.2`. It is deliberately **not** relicensed here. Relicensing is the copyright holder's prerogative, and an agent performing it silently would be asserting a grant nobody made. The workspace default is overridden for these two paths instead.

The notice travels in `LICENSES/MIT.txt`, which carries the upstream text verbatim including its copyright line — satisfying MIT's requirement that the notice accompany the copies, without vendoring two byte-identical `LICENSE` files. That last point is not incidental: the tracked-tree hygiene gate joins files by SHA-256 and would have rejected the pair, correctly.

## Consequences

The workspace builds from its own tree. `Cargo.lock` contains zero `git+` sources and no reference to `context-kit`; `deny.toml`'s `allow-git` list is empty, so `unknown-git = "deny"` now means a git dependency cannot reappear without an explicit amendment to that file.

The dependency graph shrank. `gear-loader` lost `async-trait`, `libc` and `tempfile`; `gear-memory` lost the optional `rusqlite`. The workspace's only `unsafe` FFI left with `worker.rs`.

The suite grew from 305 to 338 passing tests across 37 targets, because the vendored crates' inline unit tests now run under this repository's gates — including the two fail-closed assertions for PDF and Office extraction, which is why their fixtures were kept rather than the tests deleted.

A compilation gate can now run on a clean runner, which is the precondition it was waiting on. Two such jobs — `Rust quality gates` and `Supply-chain policy` — are proposed in `.github/workflows/rust.yml` by PR #130, which is still open at the time of writing; this decision removes the reason they were red, but it does not add the workflow and does not change branch protection. Both remain deliberate, separate steps.

## Reconsideration

If `context-kit` is ever republished, this decision does not automatically reverse. The pruning here is real divergence: an upstream that reappears would be a different, larger crate, and returning to it would mean reintroducing a git dependency, an `allow-git` exemption, and a dependency on a repository outside this one's control — the exact arrangement that failed. A republished upstream would be a reason to reconcile the two histories, not a reason to depend on it again.
