//! Synthetic rows, keys, and stores for the startup sealing backfill's cases.
//!
//! The backfill is a table-wide operation: it enumerates every legacy row in
//! `context69.internal_secrets`, not a subset these cases own. So these helpers
//! exist to make that safe rather than convenient — every synthetic row carries
//! a marker cleanup can find, and a case that needs its rows gone can delete
//! them by marker alone.
//!
//! Everything here is synthetic. The master keys are fixed test constants (byte
//! ranges `00..20` and `20..40`), the plaintexts are generated per case, and no
//! value ever names a real deployment.

use context69_secret_store::{
    SecretDatabase, SecretPurpose, SecretStore, StoredSecretBytes,
    crypto::{MasterKey, SecretCipher},
    seal_for_storage,
};
use sqlx::{PgPool, Row};

/// The configured master key for these cases, bytes `00..20`.
pub const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// A second master key, bytes `20..40`, for rows these cases did not seal.
pub const OTHER_KEY_B64: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
/// The key version [`MASTER_KEY_B64`] is registered under.
pub const KEY_VERSION: u32 = 1;
/// [`KEY_VERSION`] as the table's `integer` column holds it.
pub const KEY_VERSION_AS_INT: i32 = 1;
/// The version [`OTHER_KEY_B64`] is registered under.
pub const OTHER_KEY_VERSION: u32 = 7;
/// Marker every synthetic key name carries, so cleanup can only ever delete
/// rows these cases wrote.
pub const MARKER: &str = "c69p1sealtest";

/// A store handle over these cases' configured master key.
pub fn store(pool: &PgPool) -> SecretStore {
    build_store(pool, Some(MASTER_KEY_B64), KEY_VERSION)
}

/// A store handle with no master key: the transition state.
pub fn unkeyed_store(pool: &PgPool) -> SecretStore {
    build_store(pool, None, KEY_VERSION)
}

/// A store handle over the second key, for rows these cases did not seal.
pub fn other_store(pool: &PgPool) -> SecretStore {
    build_store(pool, Some(OTHER_KEY_B64), OTHER_KEY_VERSION)
}

fn build_store(pool: &PgPool, master_key: Option<&str>, key_version: u32) -> SecretStore {
    SecretStore::new(SecretDatabase::new(pool.clone()), master_key, key_version)
        .expect("a test master key builds a store")
}

/// A complete, catalogue-owned key name for one purpose.
///
/// Singletons get their fixed name and record-scoped purposes get their prefix
/// plus this suite's marker, so a key that resolves is always one a real
/// deployment could hold.
pub fn key_name(purpose: SecretPurpose, suffix: &str) -> String {
    match purpose.key_name_prefix() {
        Some(prefix) => format!("{prefix}{MARKER}-{suffix}"),
        None => purpose
            .singleton_key_name()
            .expect("a non record-scoped purpose has a fixed name")
            .to_string(),
    }
}

/// The synthetic plaintext for one row. Derived from the key, so a row's
/// expected recovered bytes are never a value a case has to remember.
pub fn plaintext(key: &str) -> Vec<u8> {
    format!("synthetic-plaintext-for-{key}").into_bytes()
}

/// The stored bytes a keyed store would write for one value.
///
/// Lets a case stand in for a concurrent writer *inside a transaction it already
/// holds the row lock on*: going through the store would open its own connection
/// and wait for the very lock the writer is holding.
pub fn seal(
    master_key: &str,
    key_version: u32,
    purpose: SecretPurpose,
    key: &str,
    value: &[u8],
) -> StoredSecretBytes {
    let cipher = SecretCipher::new(
        MasterKey::from_base64(master_key).expect("test master key"),
        key_version,
    );
    seal_for_storage(Some(&cipher), purpose, key, value).expect("seal the writer's value")
}

/// Writes one row in the exact legacy plaintext representation.
///
/// Deliberately raw SQL: the ordinary write path cannot produce this shape once
/// a key is configured, and the case has to place the row the transition state
/// would have written.
pub async fn insert_legacy(pool: &PgPool, key: &str, value: &[u8]) {
    sqlx::query(
        "INSERT INTO context69.internal_secrets (key, value, purpose, key_version, \
         ciphertext_version) VALUES ($1, $2, NULL, 0, 0)",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .expect("write the synthetic legacy row");
}

/// Writes a row that claims to be legacy but carries a key version.
///
/// The table's check only ties the representation marker to the owning purpose,
/// so this shape is reachable and the sweep has to meet it and refuse.
pub async fn insert_anomalous_legacy(pool: &PgPool, key: &str, value: &[u8]) {
    sqlx::query(
        "INSERT INTO context69.internal_secrets (key, value, purpose, key_version, \
         ciphertext_version) VALUES ($1, $2, NULL, 4, 0)",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .expect("write the synthetic anomalous legacy row");
}

/// One row as the table holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRow {
    /// The row key.
    pub key: String,
    /// The stored bytes.
    pub value: Vec<u8>,
    /// Owning purpose, absent on a legacy row.
    pub purpose: Option<String>,
    /// Master-key version.
    pub key_version: i32,
    /// Representation marker.
    pub ciphertext_version: i32,
    /// When the row was first written, as a UTC timestamp string.
    pub created_at: String,
    /// When a value was last written through the store, as a UTC timestamp
    /// string.
    pub updated_at: Option<String>,
}

/// Reads one row exactly as the table holds it, timestamps included.
///
/// The timestamps are projected in UTC as text: the point of the assertion is
/// that the sweep left them *identical*, and comparing one canonical rendering
/// shows that without this crate having to depend on a date-time library.
pub async fn stored_row(pool: &PgPool, key: &str) -> StoredRow {
    let row = sqlx::query(
        "SELECT key, value, purpose, key_version, ciphertext_version, \
                (created_at AT TIME ZONE 'UTC')::text AS created_at, \
                (updated_at AT TIME ZONE 'UTC')::text AS updated_at \
         FROM context69.internal_secrets WHERE key = $1",
    )
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("read the stored row");
    StoredRow {
        key: row.get("key"),
        value: row.get("value"),
        purpose: row.get("purpose"),
        key_version: row.get("key_version"),
        ciphertext_version: row.get("ciphertext_version"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// How many rows still hold the legacy plaintext representation.
pub async fn legacy_count(pool: &PgPool) -> i64 {
    sqlx::query("SELECT count(*) FROM context69.internal_secrets WHERE ciphertext_version = 0")
        .fetch_one(pool)
        .await
        .expect("count the remaining legacy rows")
        .get::<i64, _>(0)
}

/// Deletes one synthetic row, for a case that has to resolve a refusal before
/// it can retry.
pub async fn delete_key(pool: &PgPool, key: &str) {
    sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
        .bind(key)
        .execute(pool)
        .await
        .expect("delete the synthetic row");
}

/// Deletes every synthetic row these cases wrote, and nothing else.
///
/// A case that has already deleted its own rows is unaffected, so this is safe
/// to call before and after.
pub async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM context69.internal_secrets WHERE key LIKE $1 OR key = ANY($2::text[])",
    )
    .bind(format!("%{MARKER}%"))
    .bind(singleton_keys())
    .execute(pool)
    .await
    .expect("delete the synthetic rows");
}

/// Every fixed singleton name, which carries no marker of its own.
fn singleton_keys() -> Vec<String> {
    SecretPurpose::ALL
        .into_iter()
        .filter_map(SecretPurpose::singleton_key_name)
        .map(str::to_string)
        .collect()
}
