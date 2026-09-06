//! Controlled `file_library` version backfill (issue 139, phase 4).
//!
//! Facade over adjacent backfill modules. Scope stays hardcoded to
//! `file_library` in SQL; apply mode re-validates every candidate inside
//! a per-document transaction and inserts only via the existing
//! idempotent `ON CONFLICT DO NOTHING`. No helper updates
//! documents/chunks/tasks, retries jobs, migrates, or overwrites.

#[path = "document_version_backfill_types.rs"]
mod types;

#[path = "document_version_backfill_preflight.rs"]
mod preflight;

#[path = "document_version_backfill_apply.rs"]
mod apply;

pub use apply::apply_file_library_backfill;
pub use preflight::{
    check_backfill_preflight, list_missing_file_library_page, preflight_file_library_backfill,
    resolve_apply_database_url,
};
pub use types::{
    BackfillApplySummary, BackfillErrorDoc, BackfillPreflight, BackfillSkippedDoc,
    FILE_LIBRARY_BACKFILL_SOURCE_KEY, FileLibraryMissingVersion,
};
