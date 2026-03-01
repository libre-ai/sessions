//! presto-rag — ingestion, retrieval, and grounded generation for Presto-Matic.
//!
//! P1a ships the **AI provider seam** ([`provider`]): an OpenAI-compatible client
//! (Clever AI by default, BYO key/endpoint) behind a trait, with a deterministic
//! fake for tests. Ingestion into pgvector (P1b) and grounded question generation
//! (P1c) build on this seam — keeping the product decoupled from any single AI
//! vendor.

pub mod provider;
