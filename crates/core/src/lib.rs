//! presto-core — the shared client/protocol core for Presto-Matic.
//!
//! Compiled to native (via UniFFI) and to wasm (web). Holds the wire protocol
//! shared by the server and every client; the realtime client state machine and
//! Biscuit handling land with later tracer-bullet slices.

pub mod fixtures;
pub mod protocol;
