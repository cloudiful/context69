//! Round trips for every routed settings singleton, on a real migrated database.
//!
//! The checkpoint needs a disposable database round trip for each category this
//! phase moved behind the shared store, not a unit test of the same functions
//! with no rows behind them. These cases supply one per settings singleton — the
//! embedding provider key, the search/rerank key, the Docling VLM key, and the
//! runtime S3 secret key — and each one asserts the same four things against real
//! rows: the value is stored sealed and purpose-bound, the legacy plaintext column
//! is still mirrored so a rollback is lossless, a read is store-first and fails
//! closed instead of falling back, presence is answerable without the master key,
//! and the wire contract keeps its `has_*` redaction and Keep semantics.
//!
//! Fixtures live in `tests/settings_secret_store/support.rs`; one focused case file
//! per category keeps each file small enough to review on its own, and the
//! embedding category is split further into its settings round trip, its identity
//! guard, and the probe's credential, redaction and bounded-output behaviour, all
//! sharing one loopback probe endpoint.
//!
//! These tests run only when `CONTEXT69_TEST_DATABASE_URL` points at a disposable
//! migrated database; they are skipped otherwise. Every value is synthetic, no
//! case prints a stored value, and no `.env`, machine configuration, or
//! remote/shared database is used. Cleanup removes exactly the four singleton
//! settings rows and the four store rows these cases write.

#[path = "settings_secret_store/support.rs"]
mod support;

#[path = "settings_secret_store/cases_docling.rs"]
mod cases_docling;

#[path = "settings_secret_store/cases_embedding.rs"]
mod cases_embedding;

#[path = "settings_secret_store/cases_embedding_guards.rs"]
mod cases_embedding_guards;

#[path = "settings_secret_store/embedding_probe_endpoint.rs"]
mod embedding_probe_endpoint;

#[path = "settings_secret_store/cases_embedding_probe.rs"]
mod cases_embedding_probe;

#[path = "settings_secret_store/cases_embedding_probe_redaction.rs"]
mod cases_embedding_probe_redaction;

#[path = "settings_secret_store/cases_embedding_probe_bounds.rs"]
mod cases_embedding_probe_bounds;

#[path = "settings_secret_store/cases_runtime_s3.rs"]
mod cases_runtime_s3;

#[path = "settings_secret_store/cases_search.rs"]
mod cases_search;
