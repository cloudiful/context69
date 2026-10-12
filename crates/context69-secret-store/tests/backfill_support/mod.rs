//! Shared fixtures and the table-level harness for the startup sealing
//! backfill's database cases.
//!
//! The backfill is a table-wide operation: it enumerates every legacy row in
//! `context69.internal_secrets`, not a subset this suite owns. So the harness
//! exists to make that safe rather than convenient — every case runs under one
//! lock, and every synthetic row carries a marker cleanup can find.
//!
//! The cases themselves are grouped by what they are about: [`conversion`] for
//! the sweep over the rows it finds, [`refusal`] for the rows it cannot convert,
//! and [`contention`] for what happens when another startup or writer is working
//! on the same rows. Each group reads on its own through `use super::*`.

use std::sync::{Mutex, MutexGuard, PoisonError};

use context69_secret_store::{SEALED_CIPHERTEXT_VERSION, SecretPurpose};
use sqlx::{PgPool, postgres::PgPoolOptions};

mod byte_boundaries;
mod contention;
mod conversion;
mod fixtures;
mod refusal;
mod writer_after_claim;

pub use fixtures::{
    KEY_VERSION, KEY_VERSION_AS_INT, MASTER_KEY_B64, cleanup, delete_key, insert_anomalous_legacy,
    insert_legacy, key_name, legacy_count, other_store, plaintext, seal, store, stored_row,
    unkeyed_store,
};

/// One batch's worth of rows, so a case can prove a sweep really did batch.
pub const BATCH: usize = 100;

/// The purpose the two-startup race seeds its rows under.
pub const RACE_PURPOSE: SecretPurpose = SecretPurpose::GitProviderToken;
/// Two full batches, so two startups genuinely contend for work rather than one
/// of them finding the table empty.
pub const RACE_ROWS: usize = BATCH * 2;

/// Asserts every named row reads back as exactly what it held.
pub async fn assert_recovered(
    pool: &PgPool,
    purpose: SecretPurpose,
    stem: &str,
    indices: &[usize],
) {
    for index in indices {
        let key = key_name(purpose, &format!("{stem}-{index:04}"));
        assert_eq!(
            store(pool)
                .get(purpose, &key)
                .await
                .expect("open the sealed row")
                .expect("the row is stored")
                .expose(),
            plaintext(&key),
            "{key}"
        );
    }
}

/// Held for the length of every database case.
///
/// A sweep reads and rewrites the whole legacy worklist, so two cases running
/// at once would move each other's rows and report each other's counts. The
/// cases are therefore serialized rather than isolated by key name.
static TABLE: Mutex<()> = Mutex::new(());

/// Runs one case against the disposable database the runner supplied.
///
/// A missing target fails here rather than skipping: the case is `#[ignore]`d,
/// so an ordinary run never reaches it and an explicit run must actually run.
pub fn run(case: impl AsyncFnOnce(&PgPool)) {
    let _table: MutexGuard<'_, ()> = TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    sqlx::test_block_on(async {
        // The pool is opened inside the runtime that drives the case:
        // `test_block_on` is a current-thread runtime, and a pool's background
        // connection tasks belong to the runtime that spawned them.
        let pool = open().await;
        case(&pool).await;
    });
}

/// Runs a setup case, then two cases genuinely at the same time, then one
/// verification case — all under the same table lock.
///
/// Two startups really are two processes with two connection pools, so the
/// contention the backfill has to survive has to be created that way: a pool's
/// background connection tasks belong to the runtime that opened it, so a second
/// side sharing the first one's pool would hang rather than race. Each side
/// therefore opens its own pool inside its own thread, and the setup runs to
/// completion first so both sides start from the same table.
///
/// The verification case is part of this call rather than a following one on
/// purpose: the table lock is what keeps another case's `cleanup` from deleting
/// these rows between the race and the assertions about what the race left.
pub fn run_pair_after(
    setup: impl AsyncFnOnce(&PgPool),
    left: impl AsyncFnOnce(&PgPool) + Send + 'static,
    right: impl AsyncFnOnce(&PgPool) + Send + 'static,
    verify: impl AsyncFnOnce(&PgPool),
) {
    let _table: MutexGuard<'_, ()> = TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    sqlx::test_block_on(async {
        let pool = open().await;
        setup(&pool).await;
    });
    std::thread::scope(|scope| {
        scope.spawn(|| {
            sqlx::test_block_on(async {
                let pool = open().await;
                left(&pool).await;
            })
        });
        scope.spawn(|| {
            sqlx::test_block_on(async {
                let pool = open().await;
                right(&pool).await;
            })
        });
    });
    sqlx::test_block_on(async {
        let pool = open().await;
        verify(&pool).await;
    });
}

async fn open() -> PgPool {
    let url = context69_test_support::accessor::database_url();
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect the disposable test database")
}
