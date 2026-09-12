//! Integration tests for library file ingest-status sync across task
//! cancel / retry / rerun.
//!
//! Cancelling a task must mark the referenced files `cancelled` (unless the
//! file is covered by another active task or is already ingested), retrying
//! failed items and rerunning cancelled tasks must reset files to `pending`,
//! and a cancelled file must still be able to finish as `succeeded` when an
//! in-flight external request completes after the cancel.
//!
//! Cases live in `tests/task_file_status/cases_*.rs`; deduplication and
//! cross-group/concurrency behavior lives in `tests/task_file_dedup.rs`. Both
//! test crates share `tests/task_file_status/support.rs`.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

#[allow(dead_code)]
#[path = "task_file_status/support.rs"]
mod support;

#[path = "task_file_status/cases_cancel.rs"]
mod cases_cancel;

#[path = "task_file_status/cases_retry.rs"]
mod cases_retry;

#[path = "task_file_status/cases_rerun.rs"]
mod cases_rerun;
