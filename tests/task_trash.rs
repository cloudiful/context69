//! Regression tests for the task trash/restore lifecycle.
//!
//! Trashing is a soft delete of the task history row (`deleted_at`). It must
//! never touch task items, files, processed text, or vectors, must reject
//! active tasks, and must be idempotent with ownership enforced. Automatic
//! retention cleanup only purges trashed terminal history; the explicit admin
//! all-terminal purge keeps its legacy trash-agnostic semantics.
//!
//! Cases live in `tests/task_trash/cases_*.rs` with shared fixtures in
//! `tests/task_trash/support.rs`.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

#[allow(dead_code)]
#[path = "task_trash/support.rs"]
mod support;

#[path = "task_trash/cases_lifecycle.rs"]
mod cases_lifecycle;

#[path = "task_trash/cases_cleanup.rs"]
mod cases_cleanup;
