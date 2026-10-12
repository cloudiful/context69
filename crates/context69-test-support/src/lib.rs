//! Test-only support for the Context69 backend test suite.
//!
//! This crate owns the single private source of the integration-test database
//! selection ([`config`]) and the runner that drives database-backed test
//! targets ([`runner`]). It is a development dependency only and is never
//! published, so it must not be reachable from production code paths.

pub mod accessor;
pub mod config;
pub mod runner;

/// Environment variable an operator may set to select the test database
/// explicitly. It overrides the repository `.env` file.
pub const TEST_DATABASE_URL_VAR: &str = "CONTEXT69_TEST_DATABASE_URL";

/// Repository file consulted when no explicit override is present.
pub const REPO_ENV_FILE_NAME: &str = ".env";

/// Key read from [`REPO_ENV_FILE_NAME`]. An ambient `DATABASE_URL` inherited
/// from the surrounding shell never selects a target on its own.
pub const REPO_ENV_DATABASE_URL_KEY: &str = "DATABASE_URL";

/// Repository root, derived at compile time from this crate's manifest
/// directory so resolution does not depend on the caller's working directory.
#[must_use]
pub fn repo_root() -> std::path::PathBuf {
    use std::path::{Path, PathBuf};

    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
