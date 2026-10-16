//! Issue #910 P3 explicit source-generation provenance on returned chunks.
//!
//! These run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and supplies it as
//! `CONTEXT69_TEST_DATABASE_URL`; they are skipped (reported `ignored`) by a
//! plain `cargo test --workspace` and are reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.

#[path = "document_chunk_generations/support.rs"]
mod support;

#[path = "document_chunk_generations/cases.rs"]
mod cases;
