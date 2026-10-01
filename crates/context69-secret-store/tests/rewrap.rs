//! Master-key recovery and rotation: the rewrap run, against a real database.
//!
//! The rewrap is the recovery half of the secret store's key procedure, so these
//! cases exercise it where it actually runs: a migrated PostgreSQL table with
//! sealed rows under a source key. What they pin down is the contract the
//! runbook depends on — a rewrap moves every catalogue row to a strictly newer
//! key version, it is resumable because the worklist is filtered on the *source*
//! version, it preserves each row's owning purpose, it never deletes anything,
//! and everything it hands back is bounded counts.
//!
//! Every value here is synthetic: fixed test master keys (byte ranges `00..20`
//! and `20..40`), fixed synthetic plaintext, and synthetic key names under
//! record-scoped purpose prefixes. No case names a real key, and cleanup deletes
//! exactly the rows its own case wrote.
//!
//! A rewrap is a table-wide operation: it enumerates every catalogue purpose at
//! the source key version, so it needs a database whose `internal_secrets` table
//! holds nothing but its own rows. A database shared with other suites would
//! carry sealed rows this file's key cannot open, and the run would correctly
//! stop on them. The cases therefore run only when
//! `CONTEXT69_TEST_REWRAP_DATABASE_URL` names a disposable migrated database
//! this file owns; they are skipped otherwise. No `.env`, machine configuration,
//! or remote/shared database is used. The leaf crate depends on no async runtime
//! of its own, so the futures are driven with `sqlx`'s own `test_block_on` rather
//! than a test-only runtime dependency.

use std::sync::{Mutex, MutexGuard, PoisonError};

use context69_secret_store::{
    RewrapError, RewrapReport, SecretDatabase, SecretPurpose, SecretStore, SecretStoreError,
    crypto::SecretCipherError, key_names,
};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

const SOURCE_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const TARGET_KEY_B64: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
const SOURCE_VERSION: u32 = 1;
const TARGET_VERSION: u32 = 2;
const TARGET_VERSION_AS_INT: i32 = 2;

/// Held for the length of every case.
///
/// A rewrap is a table-wide maintenance operation: it enumerates every catalogue
/// purpose at the source key version, so two cases running at the same time would
/// move each other's rows and report each other's counts. The cases are therefore
/// serialized rather than isolated by key name.
static REWRAP_TABLE: Mutex<()> = Mutex::new(());

/// Runs one case against a fresh pool on the disposable database this file owns,
/// or reports that it was not pointed at one.
fn run(case: impl AsyncFnOnce(&PgPool)) {
    let url = std::env::var("CONTEXT69_TEST_REWRAP_DATABASE_URL").ok();
    let Some(url) = url.filter(|url| !url.trim().is_empty()) else {
        eprintln!(
            "CONTEXT69_TEST_REWRAP_DATABASE_URL is not set; skipping the rewrap database test"
        );
        return;
    };
    let _table: MutexGuard<'_, ()> = REWRAP_TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    sqlx::test_block_on(async move {
        // The pool is opened inside the runtime that drives the case:
        // `test_block_on` is a current-thread runtime, and a pool's background
        // connection tasks belong to the runtime that spawned them, so a pool
        // built in one block would be dead in the next.
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect the disposable test database");
        case(&pool).await;
    });
}

/// Synthetic sealed rows spanning the record-scoped purposes, one marker per
/// case. The names carry the case's own `rewraptest-<tag>-` record id, so cleanup
/// can only ever delete rows this file wrote.
fn synthetic_keys(tag: &str) -> Vec<(SecretPurpose, String)> {
    let record = format!("rewraptest-{tag}");
    [
        (SecretPurpose::SourceConnectionDatabaseUrl, "a"),
        (SecretPurpose::GitProviderToken, "b"),
        (SecretPurpose::GitWebhookSigningSecret, "c"),
        (SecretPurpose::GitHubAppPrivateKey, "d"),
    ]
    .into_iter()
    .map(|(purpose, suffix)| {
        let prefix = purpose
            .key_name_prefix()
            .expect("record-scoped purpose has a prefix");
        let key = format!("{prefix}{record}-{suffix}");
        assert!(key.contains(&record), "synthetic key {key} lost its marker");
        (purpose, key)
    })
    .collect()
}

fn store(pool: &PgPool, master_key: Option<&str>, key_version: u32) -> SecretStore {
    SecretStore::new(SecretDatabase::new(pool.clone()), master_key, key_version)
        .expect("a test master key builds a store")
}

fn target_store(pool: &PgPool) -> SecretStore {
    store(pool, Some(TARGET_KEY_B64), TARGET_VERSION)
}

/// Deletes exactly one case's synthetic rows, and nothing else.
async fn cleanup(pool: &PgPool, tag: &str) {
    for (_, key) in synthetic_keys(tag) {
        sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
            .bind(key)
            .execute(pool)
            .await
            .expect("delete the synthetic row");
    }
}

/// Writes one synthetic sealed row.
async fn write_sealed(store: &SecretStore, purpose: SecretPurpose, key: &str, value: &[u8]) {
    store
        .write(purpose, key, value)
        .await
        .expect("write the synthetic sealed row");
}

/// The stored row's metadata: owning purpose, key version, representation marker.
async fn stored_row(pool: &PgPool, key: &str) -> (Option<String>, i32, i32) {
    let row = sqlx::query(
        "SELECT purpose, key_version, ciphertext_version \
         FROM context69.internal_secrets WHERE key = $1",
    )
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("read the stored row's metadata");
    (
        row.get::<Option<String>, _>("purpose"),
        row.get::<i32, _>("key_version"),
        row.get::<i32, _>("ciphertext_version"),
    )
}

async fn row_count(pool: &PgPool) -> i64 {
    sqlx::query("SELECT count(*) FROM context69.internal_secrets")
        .fetch_one(pool)
        .await
        .expect("count the stored rows")
        .get::<i64, _>(0)
}

/// Asserts that a run refused on the key versions, and reports both of them so
/// the refusal names the ordering it rejected rather than just that it failed.
fn assert_version_refused(error: &RewrapError, source: u32, target: u32) {
    assert!(
        matches!(
            error,
            RewrapError::TargetKeyVersionNotNewer {
                source: reported,
                target: offered
            } if *reported == source && *offered == target
        ),
        "unexpected refusal: {error:?}"
    );
}

// --- success -----------------------------------------------------------------

#[test]
fn a_rewrap_moves_every_sealed_row_to_the_target_key_version() {
    run(async |pool| {
        cleanup(pool, "success").await;
        let source = store(pool, Some(SOURCE_KEY_B64), SOURCE_VERSION);
        let target = target_store(pool);
        let keys = synthetic_keys("success");
        for (index, (purpose, key)) in keys.iter().enumerate() {
            write_sealed(
                &source,
                *purpose,
                key,
                format!("synthetic-{index}").as_bytes(),
            )
            .await;
        }
        let before = row_count(pool).await;

        let report = target.rewrap_secrets_from(&source).await.expect("rewrap");

        assert_eq!(report.rewrapped, keys.len());
        assert_eq!(report.purposes, keys.len());
        assert_eq!(row_count(pool).await, before, "a rewrap deletes nothing");
        for (index, (purpose, key)) in keys.iter().enumerate() {
            let expected = format!("synthetic-{index}");
            // The target opens the row under the target key ...
            let opened = target
                .get(*purpose, key)
                .await
                .expect("open under the target key")
                .expect("the row is stored");
            assert_eq!(opened.expose(), expected.as_bytes());
            // ... and the owning purpose survived the move.
            let (row_purpose, key_version, ciphertext_version) = stored_row(pool, key).await;
            assert_eq!(row_purpose.as_deref(), Some(purpose.as_str()));
            assert_eq!(key_version, TARGET_VERSION_AS_INT);
            assert_eq!(ciphertext_version, 1, "the row is still sealed");
        }
        cleanup(pool, "success").await;
    });
}

// --- strict version ordering -------------------------------------------------

#[test]
fn a_rewrap_refuses_a_target_that_is_not_strictly_newer() {
    run(async |pool| {
        cleanup(pool, "order").await;
        let source = store(pool, Some(SOURCE_KEY_B64), SOURCE_VERSION);
        let (purpose, key) = synthetic_keys("order")[0].clone();
        write_sealed(&source, purpose, &key, b"synthetic-ordering").await;
        let before = stored_row(pool, &key).await;

        // The same version: nothing would move, so the run must not pretend to.
        let same = store(pool, Some(TARGET_KEY_B64), SOURCE_VERSION);
        assert_version_refused(
            &same
                .rewrap_secrets_from(&source)
                .await
                .expect_err("same version"),
            SOURCE_VERSION,
            SOURCE_VERSION,
        );
        // A lower version: a rewrap may never move a deployment backwards onto a
        // key it has already retired.
        let older = store(pool, Some(TARGET_KEY_B64), 0);
        assert_version_refused(
            &older
                .rewrap_secrets_from(&source)
                .await
                .expect_err("lower version"),
            SOURCE_VERSION,
            0,
        );
        // An unkeyed end cannot re-seal anything.
        assert!(matches!(
            target_store(pool)
                .rewrap_secrets_from(&store(pool, None, SOURCE_VERSION))
                .await
                .expect_err("unkeyed source"),
            RewrapError::SourceMasterKeyMissing
        ));
        assert!(matches!(
            store(pool, None, TARGET_VERSION)
                .rewrap_secrets_from(&source)
                .await
                .expect_err("unkeyed target"),
            RewrapError::TargetMasterKeyMissing
        ));

        // Every refusal happened before a row was read, so nothing moved.
        assert_eq!(stored_row(pool, &key).await, before);
        cleanup(pool, "order").await;
    });
}

// --- resumable, source-version filtered --------------------------------------

#[test]
fn a_rewrap_resumes_from_the_rows_still_at_the_source_version() {
    run(async |pool| {
        cleanup(pool, "resume").await;
        let source = store(pool, Some(SOURCE_KEY_B64), SOURCE_VERSION);
        let target = target_store(pool);
        let keys = synthetic_keys("resume");
        for (index, (purpose, key)) in keys.iter().enumerate() {
            write_sealed(
                &source,
                *purpose,
                key,
                format!("synthetic-{index}").as_bytes(),
            )
            .await;
        }
        let before = row_count(pool).await;

        // Stand in for an earlier run that stopped after the first row: that row
        // is already at the target version, the rest are still at the source one.
        let (first_purpose, first_key) = keys[0].clone();
        target
            .rotate(first_purpose, &first_key, b"synthetic-0")
            .await
            .expect("pre-rewrap one row");

        // A row the run has not reached is still readable with the outgoing key,
        // which is what keeps a deployment working until it switches over.
        let pending = source
            .get(keys[1].0, &keys[1].1)
            .await
            .expect("read the pending row")
            .expect("the pending row is stored");
        assert_eq!(pending.expose(), b"synthetic-1");

        // The run skips the row that is already at the target version.
        let report = target.rewrap_secrets_from(&source).await.expect("resume");
        assert_eq!(report.rewrapped, keys.len() - 1);
        assert_eq!(report.purposes, keys.len() - 1);
        let (_, first_version, _) = stored_row(pool, &first_key).await;
        assert_eq!(first_version, TARGET_VERSION_AS_INT);
        assert_eq!(row_count(pool).await, before, "a rewrap deletes nothing");

        // Running again is a no-op: nothing is left at the source version, and the
        // old key no longer opens what has already moved.
        let repeat = target.rewrap_secrets_from(&source).await.expect("rerun");
        assert_eq!(repeat.rewrapped, 0);
        assert_eq!(repeat.purposes, 0);
        assert!(
            matches!(
                source.get(first_purpose, &first_key).await,
                Err(SecretStoreError::Cipher(
                    SecretCipherError::KeyVersionMismatch { .. }
                ))
            ),
            "the outgoing key must fail closed on an already-rewrapped row"
        );

        // A legacy plaintext row holds no frame, so it is not the rewrap's work: it
        // keeps its representation and its value.
        let legacy_key = "source_connection.database_url.rewraptest-resume-legacy";
        write_sealed(
            &store(pool, None, SOURCE_VERSION),
            SecretPurpose::SourceConnectionDatabaseUrl,
            legacy_key,
            b"legacy-bytes",
        )
        .await;
        let after_legacy = target.rewrap_secrets_from(&source).await.expect("rerun");
        assert_eq!(after_legacy.rewrapped, 0);
        assert_eq!(stored_row(pool, legacy_key).await, (None, 0, 0));
        assert_eq!(
            source
                .get(SecretPurpose::SourceConnectionDatabaseUrl, legacy_key)
                .await
                .expect("read the legacy row")
                .expect("the legacy row is stored")
                .expose(),
            b"legacy-bytes"
        );
        sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
            .bind(legacy_key)
            .execute(pool)
            .await
            .expect("delete the synthetic legacy row");

        cleanup(pool, "resume").await;
    });
}

// --- counts only -------------------------------------------------------------

#[test]
fn a_rewrap_hands_back_counts_and_never_a_value_or_a_key_name() {
    run(async |pool| {
        cleanup(pool, "counts").await;
        let source = store(pool, Some(SOURCE_KEY_B64), SOURCE_VERSION);
        let target = target_store(pool);
        let (purpose, key) = synthetic_keys("counts")[0].clone();
        write_sealed(&source, purpose, &key, b"synthetic-countable").await;

        let report: RewrapReport = target.rewrap_secrets_from(&source).await.expect("rewrap");
        // What an operator's log line would carry, checked on the rendering: the
        // exported report is two bounded counts and nothing else.
        let rendered = format!("{report:?}");
        assert_eq!(rendered, "RewrapReport { rewrapped: 1, purposes: 1 }");
        let forbidden = [
            "synthetic-countable",
            SOURCE_KEY_B64,
            TARGET_KEY_B64,
            key.as_str(),
            purpose.as_str(),
            key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX,
        ];
        for leak in forbidden {
            assert!(!rendered.contains(leak), "{rendered} leaked {leak:?}");
        }
        // A refusal is equally free of values, key names, and key material.
        let refused = store(pool, Some(TARGET_KEY_B64), SOURCE_VERSION)
            .rewrap_secrets_from(&source)
            .await
            .expect_err("same version");
        let message = format!("{refused} {refused:?}");
        for leak in forbidden {
            assert!(!message.contains(leak), "{message} leaked {leak:?}");
        }
        cleanup(pool, "counts").await;
    });
}
