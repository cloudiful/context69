//! The shared accessor every database-backed test uses.
//!
//! Database tests must not carry their own URL lookup. They are marked
//! `#[ignore = "..."]` so the default `cargo test --workspace` run reports them
//! as ignored rather than skipping them silently, and when one is invoked
//! explicitly this accessor either hands back a validated URL or fails hard.
//! There is no `Option` and no early return.

use crate::TEST_DATABASE_URL_VAR;
use crate::config::Endpoint;

/// `#[ignore]` reason attached to every database-backed test function.
pub const DB_IGNORE_REASON: &str = "requires configured reusable PostgreSQL";

/// Opt-in gate for the application-startup service boot.
pub const APP_STARTUP_VAR: &str = "CONTEXT69_TEST_APP_STARTUP";

/// Opt-in gate for the keyword benchmark.
pub const KEYWORD_BENCHMARK_VAR: &str = "CONTEXT69_KEYWORD_BENCHMARK";

/// Whether this process was given a test database at all.
///
/// Only guard tests consult this. A database test calls [`database_url`]
/// instead, so an absent target is a failure rather than a silent pass.
#[must_use]
pub fn is_configured() -> bool {
    std::env::var(TEST_DATABASE_URL_VAR).is_ok_and(|raw| !raw.trim().is_empty())
}

/// The configured test database URL.
///
/// # Panics
///
/// Panics when the runner did not supply a target or supplied one that is not
/// a usable PostgreSQL URL. The message names only the sanitized category; it
/// never renders the value.
#[must_use]
pub fn database_url() -> String {
    let raw = std::env::var(TEST_DATABASE_URL_VAR).unwrap_or_default();
    if raw.trim().is_empty() {
        panic!(
            "{TEST_DATABASE_URL_VAR} is not set; run database-backed tests through \
             ./scripts/test-backend.sh, which resolves the configured target and supplies it"
        );
    }
    if let Err(reason) = Endpoint::parse(&raw) {
        panic!("{TEST_DATABASE_URL_VAR} is not a usable test database URL: {reason}");
    }
    raw
}

/// Whether the caller explicitly opted into the application-startup service
/// boot. These tests start real services, so they never run by default and are
/// never reported as passed by a disabled opt-in.
#[must_use]
pub fn app_startup_enabled() -> bool {
    is_enabled(APP_STARTUP_VAR)
}

/// Whether the caller explicitly opted into the keyword benchmark.
#[must_use]
pub fn keyword_benchmark_enabled() -> bool {
    is_enabled(KEYWORD_BENCHMARK_VAR)
}

fn is_enabled(key: &str) -> bool {
    std::env::var(key).is_ok_and(|raw| matches!(raw.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
}
