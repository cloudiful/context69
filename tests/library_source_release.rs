//! Issue 332 phase 2 source-lifecycle integration tests: manual release,
//! release guards, upload-time auto-release, missing-source coexistence, and
//! reprocess rejection.
//!
//! These run only when `CONTEXT69_TEST_DATABASE_URL` points at a scratch
//! database with the current migrations applied; they are skipped otherwise.
//!
//! The shared helpers live in `support_core` and the release-only additions in
//! `support_release_only`; the `support` module re-exports both so the case
//! files keep importing a single fixture namespace.

#[path = "library_source_release/support.rs"]
mod support_core;

#[path = "library_source_release/support_seed.rs"]
mod support_seed;

#[path = "library_source_release/support_seed_release.rs"]
mod support_seed_release;

#[path = "library_source_release/support_release_only.rs"]
mod support_release_only;

/// Fixture namespace imported by the case files: the shared helpers plus the
/// release-only additions.
mod support {
    pub use crate::support_core::*;
    pub use crate::support_release_only::*;
}

#[path = "library_source_release/cases_manual_release.rs"]
mod cases_manual_release;

#[path = "library_source_release/cases_guards.rs"]
mod cases_guards;

#[path = "library_source_release/cases_auto_release.rs"]
mod cases_auto_release;

#[path = "library_source_release/cases_cleanup_coexistence.rs"]
mod cases_cleanup_coexistence;

#[path = "library_source_release/cases_concurrency.rs"]
mod cases_concurrency;

#[path = "library_source_release/cases_outbox_recovery.rs"]
mod cases_outbox_recovery;
