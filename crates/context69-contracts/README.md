# context69-contracts

Shared request and response types for the Context69 HTTP API.

This crate is primarily intended for use by `context69-sdk` and the `context69` server crate.
Most Rust API consumers should depend on `context69-sdk`, which re-exports these contracts at
`context69_sdk::contracts`.
