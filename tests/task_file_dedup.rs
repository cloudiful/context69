//! Issue 332 phase 1 regressions for file processing task deduplication:
//! atomic partial-overlap rejection, idempotent reuse, cross-group
//! authorization, terminal status projection, rerun skipping, and the shared
//! create/retry/rerun per-file lock.
//!
//! Fixtures live in `tests/task_file_status/support.rs` and are shared with the
//! cancel/retry/rerun status tests. Like the other database integration tests,
//! these run only when `CONTEXT69_TEST_DATABASE_URL` is set; they are skipped
//! otherwise.

#[allow(dead_code)]
#[path = "task_file_status/support.rs"]
mod support;

#[path = "task_file_dedup/support.rs"]
mod dedup_support;

#[path = "task_file_dedup/cases_overlap.rs"]
mod cases_overlap;

#[path = "task_file_dedup/cases_ownership.rs"]
mod cases_ownership;

#[path = "task_file_dedup/cases_projection.rs"]
mod cases_projection;

#[path = "task_file_dedup/cases_rerun.rs"]
mod cases_rerun;

#[path = "task_file_dedup/cases_locks_concurrency.rs"]
mod cases_locks_concurrency;
