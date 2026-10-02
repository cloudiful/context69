//! Synthetic fixtures for the webhook registration round trips (issue #681
//! phase 5H).
//!
//! One group, one repository source, and a keyed store over the scratch pool,
//! mirroring the connection and webhook suites this phase extends. Every value is
//! generated per run: the signing secrets are synthetic strings, the store's
//! master key is a fresh random key, and no real credential, ciphertext, or
//! deployment key is read, derived, or reported here. Cleanup removes the group
//! (cascading the repository source and its registration) and the store rows the
//! phase left behind, so a run leaves the scratch database as it found it.

use uuid::Uuid;

use crate::{
    contracts::sources::{GitIndexProfile, GitProviderKind, GitRefreshPolicy},
    db::{Database, NewGitRepositorySource, StoredGitWebhookRegistration},
    services::secret_store::{SecretStore, key_names},
};

use sqlx::Row;

/// The connection key none of these fixtures uses, so a row can never be shared
/// with the connection suites.
pub(super) const UNOWNED_GROUP: i64 = 9_999_999;

/// Everything one registration round trip needs.
pub(super) struct Seeded {
    pub(super) db: Database,
    pub(super) secrets: SecretStore,
    pub(super) group_id: i64,
    pub(super) repository_key: Uuid,
}

/// The scratch database plus one group and one repository source, or `None` when
/// the round trips must skip.
pub(super) async fn seeded() -> Option<Seeded> {
    let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the create round trips");
        return None;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_key = format!("git-webhook-create-{}", Uuid::new_v4());
    let group_id: i64 = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, 'Git Webhook Create Test', 'private', 'shared', $2) RETURNING id",
    )
    .bind(&group_key)
    .bind(format!("/{group_key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id");
    let repository_key = seed_source(&db, group_id).await;
    Some(Seeded {
        secrets: store(&db),
        db,
        group_id,
        repository_key,
    })
}

/// One credential-free GitHub repository source of `group_id`, with a fresh
/// canonical URL so two seeded repositories never collide.
pub(super) async fn other_source(db: &Database, group_id: i64) -> Uuid {
    seed_source(db, group_id).await
}

/// Seeds one repository source and returns its key.
async fn seed_source(db: &Database, group_id: i64) -> Uuid {
    let nonce = Uuid::new_v4();
    db.upsert_git_repository_source(
        group_id,
        &NewGitRepositorySource {
            connection_key: None,
            provider: GitProviderKind::GitHub,
            canonical_url: format!("https://github.com/octo/{nonce}"),
            owner: "octo".to_string(),
            name: nonce.to_string(),
            default_branch: "main".to_string(),
            target_ref: "refs/heads/main".to_string(),
            target_commit_sha: None,
            index_profile: GitIndexProfile::Lexical,
            refresh_policy: GitRefreshPolicy::Manual,
        },
    )
    .await
    .expect("seed repository source")
    .repository_key
}

/// A store over the scratch pool with a per-run master key, so a sealed row in
/// this database is readable by this run and by no other.
fn store(db: &Database) -> SecretStore {
    let master_key = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        Uuid::new_v4().into_bytes().repeat(2),
    );
    SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        Some(master_key.as_str()),
        1,
    )
    .expect("a store builds from any master key")
}

/// A synthetic signing secret, distinct per call so two racing creates never
/// submit the same bytes.
pub(super) fn synthetic_secret() -> String {
    format!("synthetic-signing-secret-{}", Uuid::new_v4())
}

/// A provider-issued hook id for a synthetic repository.
pub(super) fn synthetic_hook_id() -> String {
    format!("hook-{}", Uuid::new_v4())
}

/// The group-owned registration, read back the way the route reads it.
pub(super) async fn read(
    seeded: &Seeded,
    group_id: i64,
    repository_key: Uuid,
) -> Option<StoredGitWebhookRegistration> {
    seeded
        .db
        .get_git_webhook_registration(group_id, repository_key)
        .await
        .expect("read the registration")
}

/// How many store rows exist for the webhook signing-secret purpose, so a test
/// can prove a refused duplicate wrote none. A count only: the keys themselves
/// are never selected, and the prefix is the store catalogue's own constant.
pub(super) async fn store_row_count(db: &Database) -> i64 {
    sqlx::query("SELECT count(*) AS rows FROM context69.internal_secrets WHERE key LIKE $1")
        .bind(format!("{}%", key_names::GIT_WEBHOOK_SIGNING_SECRET_PREFIX))
        .fetch_one(db.pool())
        .await
        .expect("count the store rows")
        .get("rows")
}

/// Removes the sealed rows a refused create left unreferenced for one group.
///
/// A failed create seals before it inserts, so its residue is a store row no
/// registration points at — reclaimable by the store's own cleanup, and removed
/// here by group-scoped prefix so a scratch run leaves nothing behind. The rows
/// are deleted by name pattern; no value is ever selected.
pub(super) async fn cleanup_orphans(seeded: &Seeded, group_id: i64) {
    sqlx::query("DELETE FROM context69.internal_secrets WHERE key LIKE $1")
        .bind(format!(
            "{}g{group_id}.%",
            key_names::GIT_WEBHOOK_SIGNING_SECRET_PREFIX
        ))
        .execute(seeded.db.pool())
        .await
        .expect("clean up unreferenced test store rows");
}

/// Removes the seeded group (cascading its repository source and registration)
/// and the store rows the phase sealed.
pub(super) async fn cleanup(seeded: &Seeded, store_keys: &[String]) {
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(seeded.group_id)
        .execute(seeded.db.pool())
        .await
        .expect("clean up the test group");
    for key in store_keys {
        sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
            .bind(key)
            .execute(seeded.db.pool())
            .await
            .expect("clean up a test store row");
    }
}
