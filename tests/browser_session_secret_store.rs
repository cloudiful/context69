//! The browser session signing key, round-tripped on a real migrated database.
//!
//! This key predates the encrypted store: its row name is the identity every
//! deployed installation already resolved against, so the checkpoint needs it
//! covered on real rows without starting the application. Three properties are
//! checked here, against the store seam the startup path uses:
//!
//! 1. the row keeps its historical name and owning purpose, because renaming it
//!    would orphan every installed signing key;
//! 2. the value is created once and reused by every later resolve, so two
//!    processes can never end up signing with different keys; and
//! 3. nothing about the key leaks — the stored bytes are sealed, presence is
//!    metadata-only, and every rendering an operator could see reports a length
//!    rather than a value.
//!
//! The value is synthetic and scoped to the disposable database. Cleanup deletes
//! the signing-key row only when this test is what created it, so a row that was
//! already present is left exactly as it was.
//!
//! Runs only when `CONTEXT69_TEST_DATABASE_URL` points at a disposable migrated
//! database; skipped otherwise. No `.env`, machine configuration, or
//! remote/shared database is used, and no application service is started.

use std::sync::{Mutex, PoisonError};

use context69::{
    db::Database,
    services::secret_store::{SecretPurpose, SecretStore, key_names},
};
use context69_secret_store::SecretDatabase;
use sqlx::Row;

const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const SIGNING_KEY_LEN: usize = 64;
const SYNTHETIC_KEY: &[u8; SIGNING_KEY_LEN] = &[0x5a_u8; SIGNING_KEY_LEN];

/// Held for the length of every case.
///
/// The signing key is one deployment-wide row under a fixed historical name, so
/// two cases running at the same time would resolve and rewrite each other's key.
static SIGNING_KEY_ROW: Mutex<()> = Mutex::new(());

/// The resolve path, reduced to the one store call startup makes when no session
/// key is configured: create the signing key if it is not stored, otherwise hand
/// back what the store already holds.
async fn resolve_signing_key(store: &SecretStore) -> Vec<u8> {
    store
        .get_or_create(
            SecretPurpose::BrowserSessionSigningKey,
            key_names::BROWSER_SESSION_SIGNING_KEY,
            SYNTHETIC_KEY,
        )
        .await
        .expect("resolve the browser session signing key")
        .into_inner()
}

fn store(db: &Database, master_key: Option<&str>) -> SecretStore {
    SecretStore::new(SecretDatabase::new(db.pool().clone()), master_key, 1)
        .expect("a test master key builds a store")
}

/// Runs one case against a fresh connection to the scratch database, or reports
/// that the integration environment is not configured.
///
/// The guard is taken in this synchronous frame and the awaits run in the nested
/// runtime, so the lock is never held across an await point.
fn run(case: impl AsyncFnOnce(&Database)) {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty());
    let Some(url) = url else {
        eprintln!(
            "CONTEXT69_TEST_DATABASE_URL is not set; skipping the browser session secret test"
        );
        return;
    };
    let _rows = SIGNING_KEY_ROW
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build the test runtime")
        .block_on(async move {
            let db = Database::connect(&url)
                .await
                .expect("connect test database");
            case(&db).await;
        });
}

/// Deletes the signing-key row only when this test is what created it.
async fn cleanup_created_row(db: &Database, existed_before: bool) {
    if existed_before {
        return;
    }
    sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
        .bind(key_names::BROWSER_SESSION_SIGNING_KEY)
        .execute(db.pool())
        .await
        .expect("delete the signing-key row this test created");
}

async fn stored_bytes(db: &Database) -> Option<(Option<String>, i32, i32, Vec<u8>)> {
    let row = sqlx::query(
        "SELECT purpose, key_version, ciphertext_version, value \
         FROM context69.internal_secrets WHERE key = $1",
    )
    .bind(key_names::BROWSER_SESSION_SIGNING_KEY)
    .fetch_optional(db.pool())
    .await
    .expect("read the signing-key row");
    row.map(|row| {
        (
            row.get::<Option<String>, _>("purpose"),
            row.get::<i32, _>("key_version"),
            row.get::<i32, _>("ciphertext_version"),
            row.get::<Vec<u8>, _>("value"),
        )
    })
}

#[test]
fn the_signing_key_row_keeps_its_historical_name_and_is_reused_across_resolves() {
    run(async |db| {
        let existed_before = stored_bytes(db).await.is_some();
        let keyed = store(db, Some(MASTER_KEY_B64));

        // The name is the identity every installed signing key already resolves
        // against: renaming it would orphan them all, so it is pinned here and not
        // derived from anything in this build.
        assert_eq!(
            key_names::BROWSER_SESSION_SIGNING_KEY,
            "browser_session_signing_key_v2"
        );
        assert_eq!(
            SecretPurpose::BrowserSessionSigningKey.as_str(),
            "browser_session.signing_key"
        );

        let first = resolve_signing_key(&keyed).await;
        let after_first = stored_bytes(db).await;
        let second = resolve_signing_key(&keyed).await;
        let after_second = stored_bytes(db).await;

        // Same key on every resolve: the candidate is only ever a seed, so a second
        // process cannot sign with a different key than the first one.
        assert_eq!(first.len(), SIGNING_KEY_LEN);
        assert_eq!(first, second, "a second resolve must reuse the stored key");
        assert_eq!(
            after_second.map(|(_, _, _, bytes)| bytes),
            after_first.map(|(_, _, _, bytes)| bytes),
            "a second resolve must not rewrite the row"
        );

        cleanup_created_row(db, existed_before).await;
    });
}

#[test]
fn the_signing_key_is_sealed_and_leaks_neither_its_bytes_nor_its_presence() {
    run(async |db| {
        let existed_before = stored_bytes(db).await.is_some();
        let keyed = store(db, Some(MASTER_KEY_B64));

        let resolved = keyed
            .get_or_create(
                SecretPurpose::BrowserSessionSigningKey,
                key_names::BROWSER_SESSION_SIGNING_KEY,
                SYNTHETIC_KEY,
            )
            .await
            .expect("resolve the signing key");
        let (purpose, key_version, ciphertext_version, stored) = stored_bytes(db)
            .await
            .expect("the signing-key row is stored");

        assert_eq!(purpose.as_deref(), Some("browser_session.signing_key"));
        assert_eq!(key_version, 1);
        assert_eq!(
            ciphertext_version, 1,
            "the stored row is sealed, not plaintext"
        );
        assert!(
            !stored
                .windows(SIGNING_KEY_LEN)
                .any(|window| window == SYNTHETIC_KEY),
            "the stored bytes must not contain the signing key"
        );
        assert!(
            !stored
                .windows(MASTER_KEY_B64.len())
                .any(|window| window == MASTER_KEY_B64.as_bytes()),
            "the stored bytes must not contain the master key"
        );

        // A resolved value renders as its length only, so an operator-facing log line
        // or error chain can mention it without disclosing it.
        assert_eq!(format!("{resolved:?}"), "SecretValue { len: 64, .. }");

        // Presence is metadata-only, so a deployment that cannot open the key still
        // knows it is configured; opening it is what fails, with a message that names
        // the configuration input and no key material.
        let unkeyed = store(db, None);
        assert!(
            unkeyed
                .has(
                    SecretPurpose::BrowserSessionSigningKey,
                    key_names::BROWSER_SESSION_SIGNING_KEY
                )
                .await
                .expect("presence needs no master key")
        );
        let refused = unkeyed
            .get(
                SecretPurpose::BrowserSessionSigningKey,
                key_names::BROWSER_SESSION_SIGNING_KEY,
            )
            .await
            .expect_err("a sealed signing key must not be readable without a master key");
        let message = format!("{refused} {refused:?}");
        assert!(message.contains("secret_store.master_key"), "{message}");
        assert!(!message.contains(MASTER_KEY_B64), "{message}");

        cleanup_created_row(db, existed_before).await;
    });
}
