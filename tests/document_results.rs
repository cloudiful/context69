//! Issue #910 P1 caller-produced document result persistence tests.
//!
//! These run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and supplies it as
//! `CONTEXT69_TEST_DATABASE_URL`; they are skipped (reported `ignored`) by a
//! plain `cargo test --workspace` and are reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.

#[path = "document_results/support.rs"]
mod support;

#[path = "document_results/cases.rs"]
mod cases;
