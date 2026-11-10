//! Refresh convergence, single-flight admission, and retention/disposal
//! behavior (issue #28 phase P4).
//!
//! Every case runs against the migrated scratch database named by
//! `CONTEXT69_TEST_DATABASE_URL`. They exercise the durable demand marker
//! shared by the scheduler, manual sync, and webhook dispatch, the per-source
//! admission fence, and vector cleanup failures that must retain ownership for
//! a retry. Groups created here are deleted with their cascade, and the one
//! task user is removed explicitly.

mod deletion;
mod ownership;
mod retention;
mod scheduling;
mod support;
