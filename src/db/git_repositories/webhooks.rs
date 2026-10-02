//! Git webhook registration and delivery persistence (issue #681 work unit
//! 3B1).
//!
//! Registrations hang off a repository source, so they are group-scoped
//! through that join. Deliveries are keyed by the provider delivery id and are
//! recorded before a repository is necessarily known, so they stay
//! provider-scoped; group resolution happens when a delivery is dispatched.
//!
//! An unauthenticated provider ingress knows only the provider and the hook id,
//! so one read resolves a registration by that unique pair and returns the
//! owning group id beside it — the minimum an ingress needs to derive the
//! record-scoped signing-secret name without trusting a caller-supplied group.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::rows::{GitWebhookDeliveryRow, GitWebhookRegistrationRow};
use super::types::{
    NewGitWebhookDelivery, NewGitWebhookRegistration, StoredGitWebhookDelivery,
    StoredGitWebhookRegistration,
};
use crate::contracts::sources::{GitProviderKind, GitWebhookDeliveryStatus};
use crate::db::Database;

/// The columns of a registration read by provider and external hook id, plus
/// the owning group id. The group id is not part of
/// [`StoredGitWebhookRegistration`] because a group-scoped caller already knows
/// it; an ingress that resolves the hook from its provider-facing id does not,
/// and needs it to derive the record-scoped signing-secret name.
#[derive(Debug, sqlx::FromRow)]
struct GitWebhookRegistrationByHookRow {
    group_id: i64,
    repository_key: Uuid,
    provider_kind: String,
    external_hook_id: String,
    ownership: String,
    active: bool,
    signing_secret_key: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Database {
    /// Begins the transaction that serializes registration creation for one
    /// repository of `group_id`.
    ///
    /// Takes a transaction-scoped advisory lock keyed by the owning group and the
    /// repository the registration hangs from as its own statement, so two
    /// concurrent creates for the same repository queue behind each other. The
    /// caller must run the group-scoped pre-check, the seal-only secret write, and
    /// the create-only insert while holding it, then
    /// [`GitWebhookRegistrationCreation::commit`] or
    /// [`GitWebhookRegistrationCreation::rollback`] explicitly: the lock is
    /// released only when that transaction ends.
    ///
    /// Serializing the whole create is what bounds the deterministic-secret race
    /// the seal split would otherwise leave open: two creates for one repository
    /// cannot both seal before either inserts, so the losing request blocks before
    /// it can overwrite the sealed row the winner references, and the value the
    /// winner stored is the winner's own.
    pub async fn begin_git_webhook_registration_creation(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<GitWebhookRegistrationCreation<'static>> {
        let mut tx = self.pool.begin().await?;
        let lock_key =
            format!("context69.git_webhook_registration_creation:{group_id}:{repository_key}");
        sqlx::query_file!(
            "src/sql/db/git_repositories/acquire_git_webhook_registration_creation_lock.sql",
            lock_key
        )
        .execute(&mut *tx)
        .await?;
        Ok(GitWebhookRegistrationCreation { tx })
    }

    /// Registers or updates the webhook of a repository source owned by
    /// `group_id`, reporting an error when the source belongs to another group.
    pub async fn upsert_git_webhook_registration(
        &self,
        group_id: i64,
        registration: &NewGitWebhookRegistration,
    ) -> Result<StoredGitWebhookRegistration> {
        let row = sqlx::query_file_as!(
            GitWebhookRegistrationRow,
            "src/sql/db/git_repositories/upsert_git_webhook_registration.sql",
            registration.repository_key,
            group_id,
            registration.provider.as_str(),
            registration.external_hook_id,
            registration.ownership.as_str(),
            registration.active,
            registration.signing_secret_key
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("no git repository source of this group to attach a webhook to")
        })?;
        StoredGitWebhookRegistration::from_row(row)
    }

    /// Reads the webhook registration of a repository source owned by
    /// `group_id`; a source of another group reads as absent.
    pub async fn get_git_webhook_registration(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Option<StoredGitWebhookRegistration>> {
        get_git_webhook_registration_on(&self.pool, group_id, repository_key).await
    }

    /// Resolves a webhook registration by its provider-facing identity, plus
    /// the owning group id.
    ///
    /// The unique `(provider_kind, external_hook_id)` index makes this zero or
    /// one row, and no group is part of the lookup: a caller that only holds the
    /// provider's hook id cannot learn which group owns it except by presenting
    /// the signature the record's secret verifies.
    pub async fn get_git_webhook_registration_by_hook(
        &self,
        provider: GitProviderKind,
        external_hook_id: &str,
    ) -> Result<Option<(i64, StoredGitWebhookRegistration)>> {
        let row = sqlx::query_file_as!(
            GitWebhookRegistrationByHookRow,
            "src/sql/db/git_repositories/get_git_webhook_registration_by_hook.sql",
            provider.as_str(),
            external_hook_id
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let registration = StoredGitWebhookRegistration::from_row(GitWebhookRegistrationRow {
                repository_key: row.repository_key,
                provider_kind: row.provider_kind,
                external_hook_id: row.external_hook_id,
                ownership: row.ownership,
                active: row.active,
                signing_secret_key: row.signing_secret_key,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })?;
            Ok((row.group_id, registration))
        })
        .transpose()
    }

    /// Narrows the signing-secret reference of one webhook registration owned by
    /// `group_id`, reporting whether a row matched.
    ///
    /// Group ownership is resolved through the registration's repository source,
    /// so a registration of another group's repository matches no row. `active`
    /// is preserved: sealing a signing secret is not a lifecycle change.
    pub async fn set_git_webhook_signing_secret_key(
        &self,
        group_id: i64,
        repository_key: Uuid,
        secret_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/git_repositories/set_git_webhook_signing_secret_key.sql",
            group_id,
            repository_key,
            secret_key
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Records a provider delivery, returning the stored row. A redelivery of
    /// the same `delivery_id` inserts nothing and returns the original row, so
    /// callers can compare `received_at` to detect the duplicate.
    pub async fn record_git_webhook_delivery(
        &self,
        delivery: &NewGitWebhookDelivery,
    ) -> Result<StoredGitWebhookDelivery> {
        let row = sqlx::query_file_as!(
            GitWebhookDeliveryRow,
            "src/sql/db/git_repositories/record_git_webhook_delivery.sql",
            delivery.delivery_id,
            delivery.provider.as_str(),
            delivery.repository_key,
            delivery.status.as_str(),
            delivery.target_commit_sha
        )
        .fetch_one(&self.pool)
        .await?;
        StoredGitWebhookDelivery::from_row(row)
    }

    pub async fn mark_git_webhook_delivery_processed(
        &self,
        delivery_id: &str,
        status: GitWebhookDeliveryStatus,
        target_commit_sha: Option<&str>,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_webhook_delivery_processed.sql",
            delivery_id,
            status.as_str(),
            target_commit_sha
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }
}

/// The transaction that serializes creation of one registration.
///
/// The caller never names this type: it is the value
/// [`Database::begin_git_webhook_registration_creation`] returns, and the lock it
/// holds is released only by the explicit [`Self::commit`] or [`Self::rollback`].
pub struct GitWebhookRegistrationCreation<'a> {
    tx: Transaction<'a, Postgres>,
}

impl GitWebhookRegistrationCreation<'_> {
    /// Reads the group-owned registration inside the locked transaction, so the
    /// pre-check sees a winner that committed before the lock was taken.
    pub async fn get(
        &mut self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Option<StoredGitWebhookRegistration>> {
        get_git_webhook_registration_on(&mut *self.tx, group_id, repository_key).await
    }

    /// Runs the create-only insert inside the locked transaction.
    ///
    /// The insert has no conflict clause, so a repository that already has a
    /// registration and a hook identity another repository already claims both
    /// raise their unique violation for the caller to map to one bounded conflict.
    pub async fn insert(
        &mut self,
        group_id: i64,
        registration: &NewGitWebhookRegistration,
    ) -> Result<StoredGitWebhookRegistration> {
        insert_git_webhook_registration_on(&mut *self.tx, group_id, registration).await
    }

    /// Commits and releases the lock.
    pub async fn commit(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }

    /// Rolls back and releases the lock.
    pub async fn rollback(self) -> Result<()> {
        self.tx.rollback().await?;
        Ok(())
    }
}

/// Reads one registration through any executor, so the plain pool read and the
/// locked transaction read share one statement and one shape.
async fn get_git_webhook_registration_on<'e, E>(
    executor: E,
    group_id: i64,
    repository_key: Uuid,
) -> Result<Option<StoredGitWebhookRegistration>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_file_as!(
        GitWebhookRegistrationRow,
        "src/sql/db/git_repositories/get_git_webhook_registration.sql",
        group_id,
        repository_key
    )
    .fetch_optional(executor)
    .await?;
    row.map(StoredGitWebhookRegistration::from_row).transpose()
}

/// Runs the create-only insert through any executor, so the locked transaction is
/// the only way the create path writes a registration.
async fn insert_git_webhook_registration_on<'e, E>(
    executor: E,
    group_id: i64,
    registration: &NewGitWebhookRegistration,
) -> Result<StoredGitWebhookRegistration>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_file_as!(
        GitWebhookRegistrationRow,
        "src/sql/db/git_repositories/insert_git_webhook_registration.sql",
        registration.repository_key,
        group_id,
        registration.provider.as_str(),
        registration.external_hook_id,
        registration.ownership.as_str(),
        registration.active,
        registration.signing_secret_key
    )
    .fetch_one(executor)
    .await?;
    StoredGitWebhookRegistration::from_row(row)
}

#[cfg(test)]
mod signing_secret_reference_tests {
    //! The narrow signing-secret statement against a disposable database.
    //!
    //! Group ownership of a registration comes from its repository source, so the
    //! two properties worth pinning are that the update is confined through that
    //! join and that it leaves the registration's `active` state alone.

    use crate::{
        contracts::sources::{
            GitIndexProfile, GitProviderKind, GitRefreshPolicy, GitWebhookOwnership,
        },
        db::{
            Database, NewGitRepositorySource, NewGitWebhookRegistration,
            StoredGitWebhookRegistration,
        },
        services::{
            git_secrets::{GitSecretSlot, GitSecretTarget, GitSecretWrite, GitSecretWriter},
            secret_store::{SecretPurpose, SecretStore},
        },
    };
    use sqlx::Row;
    use uuid::Uuid;

    /// A group id that owns no repository source, so an update matches no row.
    const UNOWNED_GROUP: i64 = 9_999_999;

    /// One group, one repository, and an inactive registration on it, plus a
    /// keyed store over the scratch pool. Skipped unless
    /// `CONTEXT69_TEST_DATABASE_URL` names a migrated disposable database.
    async fn fixture() -> Option<(Database, i64, Uuid, SecretStore)> {
        let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        let key = format!("git-webhook-secret-{}", Uuid::new_v4());
        let group_id: i64 = sqlx::query(
            "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
             VALUES ($1, 'Git Webhook Test', 'private', 'shared', $2) RETURNING id",
        )
        .bind(&key)
        .bind(format!("/{key}"))
        .fetch_one(db.pool())
        .await
        .expect("seed group")
        .get("id");
        let nonce = Uuid::new_v4();
        let repository_key = db
            .upsert_git_repository_source(
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
            .repository_key;
        db.upsert_git_webhook_registration(
            group_id,
            &NewGitWebhookRegistration {
                repository_key,
                provider: GitProviderKind::GitHub,
                external_hook_id: nonce.to_string(),
                ownership: GitWebhookOwnership::Integration,
                active: false,
                signing_secret_key: None,
            },
        )
        .await
        .expect("seed webhook registration");
        // A per-run key, so nothing here is a credential of any deployment.
        let master_key = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            Uuid::new_v4().into_bytes().repeat(2),
        );
        Some((
            db.clone(),
            group_id,
            repository_key,
            SecretStore::new(
                context69_secret_store::SecretDatabase::new(db.pool().clone()),
                Some(master_key.as_str()),
                1,
            )
            .expect("a store builds from any master key"),
        ))
    }

    fn target(group_id: i64, repository_key: Uuid) -> GitSecretTarget {
        GitSecretTarget::Repository {
            group_id,
            repository_key,
        }
    }

    async fn read(
        db: &Database,
        group_id: i64,
        repository_key: Uuid,
    ) -> StoredGitWebhookRegistration {
        db.get_git_webhook_registration(group_id, repository_key)
            .await
            .expect("read the registration")
            .expect("the registration is still there")
    }

    /// A signing secret reaches the registration's own reference under its own
    /// purpose, an inactive registration stays inactive, and a foreign group
    /// neither reaches the reference nor disturbs the sealed value it left
    /// behind.
    #[tokio::test]
    async fn a_signing_secret_moves_its_own_reference_and_preserves_active() {
        let Some((db, group_id, repository_key, store)) = fixture().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the webhook round trip");
            return;
        };
        let writer = GitSecretWriter::new(db.clone(), store.clone());
        let key_name = match writer
            .write(
                &target(group_id, repository_key),
                GitSecretSlot::Webhook,
                Some(b"hook-bytes"),
            )
            .await
            .expect("seal the signing secret")
        {
            GitSecretWrite::Stored(name) => name.as_str().to_string(),
            GitSecretWrite::Kept => panic!("a value is stored, not kept"),
        };

        let row = read(&db, group_id, repository_key).await;
        assert_eq!(row.signing_secret_key.as_deref(), Some(key_name.as_str()));
        assert!(
            !row.active,
            "a rotation must not re-activate a hook an operator deactivated"
        );
        assert_eq!(
            store
                .get(SecretPurpose::GitWebhookSigningSecret, &key_name)
                .await
                .expect("open the sealed value")
                .expect("it is stored")
                .expose(),
            &b"hook-bytes"[..],
            "the signing secret is in the store immediately, under its own purpose"
        );

        // A write aimed at a group that owns no such repository matches no row.
        // The bounded failure is an orphan, never a loss.
        let error = writer
            .write(
                &target(UNOWNED_GROUP, repository_key),
                GitSecretSlot::Webhook,
                Some(b"orphan-bytes"),
            )
            .await
            .expect_err("no registration of that group is updated");
        let message = error.to_string();
        assert!(
            message.contains("git_webhook.signing_secret")
                && message.contains("git_webhook_registrations.signing_secret_key")
                && !message.contains("orphan-bytes")
                && !message.contains("hook-bytes"),
            "{message}"
        );
        let survivor = read(&db, group_id, repository_key).await;
        assert_eq!(
            survivor.signing_secret_key.as_deref(),
            Some(key_name.as_str()),
            "a failed reference update never clears the reference already in place"
        );
        assert!(!survivor.active);
    }
}
