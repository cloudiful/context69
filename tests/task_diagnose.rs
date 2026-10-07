//! Task diagnose and consistency-projection regressions (issue 702 P3).
//!
//! `task_items` is the execution-state source of truth and every parent counter
//! is its projection, so the diagnose verdict and the `/healthz` gauges must
//! agree with the item rows in the same read. These tests seed the three states
//! an operator has to be able to tell apart — a fresh task, a running claim,
//! and a skewed parent — and assert the verdict names the breach instead of
//! hiding it.
//!
//! Split into cohesive modules: shared fixtures in `support`, verdict cases in
//! `cases_verdict`, gauge cases in `cases_gauges`, and ordinal-resolution cases
//! in `cases_ordinal`. All DB-backed cases take the one `SUITE_LOCK` in
//! `support`, so they still serialize against each other. They run only when
//! `CONTEXT69_TEST_DATABASE_URL` points to a scratch database (migrations are
//! applied automatically) and are skipped otherwise.

#[path = "task_diagnose/support.rs"]
mod support;

#[path = "task_diagnose/cases_verdict.rs"]
mod cases_verdict;

#[path = "task_diagnose/cases_gauges.rs"]
mod cases_gauges;

#[path = "task_diagnose/cases_ordinal.rs"]
mod cases_ordinal;
