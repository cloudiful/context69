//! Snapshot indexing tests (issue #681 work unit 3B3).
//!
//! Orchestration runs against the real group-scoped repository, generation,
//! and content methods with a scripted in-memory acquirer, so no live provider
//! is contacted. Persistence cases connect through the crate's standard test
//! constructor to the already-migrated scratch database named by
//! `CONTEXT69_TEST_DATABASE_URL`, author no migration, and delete their own
//! group. Each test reads its state before cleanup, so a failing assertion
//! leaves no residue.

mod acquisition;
mod fencing;
mod incremental;
mod snapshots;
mod support;
mod vectors;
