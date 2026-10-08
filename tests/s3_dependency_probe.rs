//! S3 dependency-gate recovery (issue 702 P2).
//!
//! The S3 gate is tripped by storage failures but no ordinary storage call can
//! lift it: operations refuse to touch the backend while the gate is not closed.
//! These cases cover the recovery model end to end:
//!
//! - a due probe closes an open gate after a successful backend check,
//! - a failed probe reopens the gate with exponential backoff,
//! - a `configuration:` pin is not probeable (the fingerprint must change),
//! - a probe before the backoff elapses is not due,
//! - a non-S3 backend never touches the gate.
//!
//! Split into cohesive modules: shared fixtures in `support`, state-machine
//! cases in `cases_state_machine`, and no-op isolation cases in
//! `cases_isolation`. Every case mutates the singleton `s3` gate row and takes
//! the one `SUITE_LOCK` in `support`. DB-backed cases run only when
//! `CONTEXT69_TEST_DATABASE_URL` is set; the unreachable S3 endpoint is
//! loopback-only, so a probe can never reach an external service.

#[path = "s3_dependency_probe/support.rs"]
mod support;

#[path = "s3_dependency_probe/cases_state_machine.rs"]
mod cases_state_machine;

#[path = "s3_dependency_probe/cases_isolation.rs"]
mod cases_isolation;
