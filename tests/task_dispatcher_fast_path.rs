//! Regression tests for the Phase 3 claim-hot-path split.
//!
//! `claim_items` (compatibility path) keeps the old behavior: maintenance
//! and fast claim run in one transaction, exhausted items become terminal,
//! and expired attempts for the items being claimed are recycled.
//! `claim_items_fast` and `maintain_claim_state` are the dispatcher-side
//! primitives that split the same work across two separate statements so
//! notification-driven wakes skip the maintenance CTEs. These tests pin
//! the seam at the database boundary (no live dispatcher) so the contract
//! is regression-safe even when the worker pool wiring changes.
//!
//! Like the other integration tests, these run only when
//! `CONTEXT69_TEST_DATABASE_URL` is set; they are skipped otherwise.

#[path = "task_dispatcher_fast_path/support.rs"]
mod support;

#[path = "task_dispatcher_fast_path/cases_fast_claim.rs"]
mod cases_fast_claim;

#[path = "task_dispatcher_fast_path/cases_maintenance_exhausted.rs"]
mod cases_maintenance_exhausted;

#[path = "task_dispatcher_fast_path/cases_maintenance_expired.rs"]
mod cases_maintenance_expired;

#[path = "task_dispatcher_fast_path/cases_compatibility.rs"]
mod cases_compatibility;

#[path = "task_dispatcher_fast_path/cases_outcome.rs"]
mod cases_outcome;
