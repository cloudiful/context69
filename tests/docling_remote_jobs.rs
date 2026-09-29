//! Durable Docling remote-job references for the blocking worker flow
//! (issue #650 P3).
//!
//! The remote row is a crash-recovery reference, not a queue: the worker
//! submits, polls, and fetches inline holding its item lease, and the
//! recovery pass only fences orphaned rows and adopts pre-P3 parked items.
//! Split per reviewer note 9599: shared fixtures live in `support`, each case
//! file stays cohesive and under 300 lines. All assertions are scoped by id,
//! never by global counts, so leftover rows in the shared scratch DB cannot
//! flake the suite.

#[path = "docling_remote_jobs/support.rs"]
mod support;

#[path = "docling_remote_jobs/cases_basic.rs"]
mod cases_basic;

#[path = "docling_remote_jobs/cases_terminal.rs"]
mod cases_terminal;

#[path = "docling_remote_jobs/cases_legacy_adoption.rs"]
mod cases_legacy_adoption;

#[path = "docling_remote_jobs/cases_recovery.rs"]
mod cases_recovery;
