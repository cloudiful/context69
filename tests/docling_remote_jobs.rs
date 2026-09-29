//! Durable Docling remote-job persistence and poll sweep (issue #639).
//!
//! Split per reviewer note 9599: shared fixtures live in `support`, each case
//! file stays cohesive and under 300 lines. All assertions are scoped by id,
//! never by global counts, so leftover rows in the shared scratch DB cannot
//! flake the suite.

#[path = "docling_remote_jobs/support.rs"]
mod support;

#[path = "docling_remote_jobs/cases_basic.rs"]
mod cases_basic;

#[path = "docling_remote_jobs/cases_claim.rs"]
mod cases_claim;

#[path = "docling_remote_jobs/cases_terminal.rs"]
mod cases_terminal;

#[path = "docling_remote_jobs/cases_sweep.rs"]
mod cases_sweep;

#[path = "docling_remote_jobs/cases_atomicity.rs"]
mod cases_atomicity;
