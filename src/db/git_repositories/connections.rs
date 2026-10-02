//! Group-scoped provider connection persistence (issue #681 work unit 3B1).
//!
//! Every operation takes the owning group: reads filter on it, and upserts
//! conflict on `(group_id, connection_key)`, so one group can neither read nor
//! take over another group's connection.

use anyhow::Result;
use sqlx::{Postgres, Transaction};

use super::rows::GitProviderConnectionRow;
use super::types::{NewGitProviderConnection, StoredGitProviderConnection};
use crate::db::Database;

impl Database {
    /// Registers or updates a provider connection owned by `group_id`.
    ///
    /// Secret columns carry internal secret-store keys only; the stored record
    /// never holds credential or signing material.
    pub async fn upsert_git_provider_connection(
        &self,
        group_id: i64,
        connection: &NewGitProviderConnection,
    ) -> Result<StoredGitProviderConnection> {
        let row = sqlx::query_file_as!(
            GitProviderConnectionRow,
            "src/sql/db/git_repositories/upsert_git_provider_connection.sql",
            group_id,
            connection.connection_key,
            connection.provider.as_str(),
            connection.mode.as_str(),
            connection.display_name,
            connection.base_url,
            connection.credential_secret_key,
            connection.webhook_secret_key
        )
        .fetch_one(&self.pool)
        .await?;
        StoredGitProviderConnection::from_row(row)
    }

    /// Creates a provider connection owned by `group_id` as a create-only
    /// insert.
    ///
    /// Unlike [`Database::upsert_git_provider_connection`] this statement has no
    /// conflict clause, so a duplicate `(group_id, connection_key)` raises the
    /// unique violation the caller maps to a bounded conflict instead of
    /// overwriting or re-enabling an existing connection. `disabled_at` is
    /// never written, so a new row is enabled, and secret columns carry
    /// internal secret-store keys only.
    pub async fn insert_git_provider_connection(
        &self,
        group_id: i64,
        connection: &NewGitProviderConnection,
    ) -> Result<StoredGitProviderConnection> {
        insert_git_provider_connection_on(self.pool(), group_id, connection).await
    }

    /// Reads one connection owned by `group_id`; a connection of another group
    /// reads as absent.
    pub async fn get_git_provider_connection(
        &self,
        group_id: i64,
        connection_key: &str,
    ) -> Result<Option<StoredGitProviderConnection>> {
        get_git_provider_connection_on(self.pool(), group_id, connection_key).await
    }

    /// Begins the transaction that serializes creation of one
    /// `(group_id, connection_key)`.
    ///
    /// Takes a transaction-scoped PostgreSQL advisory lock keyed by the owning
    /// group and the connection key as its own statement, so every concurrent
    /// create of the same key queues behind the one already inside. The caller
    /// must run the pre-check, the seal-only secret write, and the create-only
    /// insert while holding this transaction, then [`GitConnectionCreation::commit`]
    /// or [`GitConnectionCreation::rollback`] explicitly: the lock is released
    /// only when that transaction ends.
    ///
    /// Serializing the whole create bounds the deterministic-secret race the
    /// seal split leaves open: two creates for one new key cannot both seal
    /// before either inserts, so a losing request blocks before it can rotate
    /// the encrypted row the winner references. It never seals, and the winner's
    /// stored value is the winner's own.
    pub async fn begin_git_connection_creation(
        &self,
        group_id: i64,
        connection_key: &str,
    ) -> Result<GitConnectionCreation<'static>> {
        let mut tx = self.pool.begin().await?;
        let lock_key = format!("context69.git_connection_creation:{group_id}:{connection_key}");
        sqlx::query_file!(
            "src/sql/db/git_repositories/acquire_git_connection_creation_lock.sql",
            lock_key
        )
        .execute(&mut *tx)
        .await?;
        Ok(GitConnectionCreation { tx })
    }

    /// Lists the connections of one group only.
    pub async fn list_git_provider_connections(
        &self,
        group_id: i64,
    ) -> Result<Vec<StoredGitProviderConnection>> {
        let rows = sqlx::query_file_as!(
            GitProviderConnectionRow,
            "src/sql/db/git_repositories/list_git_provider_connections.sql",
            group_id
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(StoredGitProviderConnection::from_row)
            .collect()
    }

    /// Narrows the read-credential reference of one connection owned by
    /// `group_id`, reporting whether a row matched.
    ///
    /// The broad connection upsert is not reused here: its conflict clause
    /// clears `disabled_at`, so rotating a credential through it would re-enable
    /// a disabled connection. This statement moves the reference alone.
    pub async fn set_git_connection_credential_secret_key(
        &self,
        group_id: i64,
        connection_key: &str,
        secret_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/git_repositories/set_git_connection_credential_secret_key.sql",
            group_id,
            connection_key,
            secret_key
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Narrows the GitHub App private-key reference of one connection owned by
    /// `group_id`, reporting whether a row matched.
    ///
    /// Separate from the credential reference because the App key is a distinct
    /// secret with a distinct purpose, and separate from the broad upsert for
    /// the same `disabled_at` reason as the credential reference.
    pub async fn set_git_connection_app_private_key_secret_key(
        &self,
        group_id: i64,
        connection_key: &str,
        secret_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/git_repositories/set_git_connection_app_private_key_secret_key.sql",
            group_id,
            connection_key,
            secret_key
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Restores a connection owned by `group_id` to service, reporting whether a
    /// row matched. A connection of another group never matches.
    ///
    /// The statement clears only the lifecycle column and stamps `updated_at`: the
    /// stored credential, App-private-key, and webhook-signing-secret references
    /// are left untouched, so an enable/disable cycle can neither rotate nor lose
    /// a secret, and nothing here reaches the secret store. It does not require the
    /// connection to be disabled, so enabling an already-enabled connection is the
    /// same successful match rather than a conflict.
    pub async fn enable_git_provider_connection(
        &self,
        group_id: i64,
        connection_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/git_repositories/enable_git_provider_connection.sql",
            group_id,
            connection_key
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Disables a connection owned by `group_id`, reporting whether a row
    /// matched. A connection of another group never matches.
    pub async fn disable_git_provider_connection(
        &self,
        group_id: i64,
        connection_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/git_repositories/disable_git_provider_connection.sql",
            group_id,
            connection_key
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

/// The create transaction that holds the per-key creation lock.
///
/// It is the only writer allowed to reach the pre-check, the seal-only secret
/// write, and the create-only insert for its key, so a concurrent create either
/// observes the committed row or waits. The lock is transaction-scoped and is
/// released by the explicit [`GitConnectionCreation::commit`] or
/// [`GitConnectionCreation::rollback`].
pub struct GitConnectionCreation<'a> {
    tx: Transaction<'a, Postgres>,
}

impl GitConnectionCreation<'_> {
    /// Reads one connection owned by `group_id` inside the locked transaction,
    /// so the pre-check sees a winner that committed before the lock was taken.
    pub async fn get(
        &mut self,
        group_id: i64,
        connection_key: &str,
    ) -> Result<Option<StoredGitProviderConnection>> {
        get_git_provider_connection_on(&mut *self.tx, group_id, connection_key).await
    }

    /// Runs the create-only insert inside the locked transaction.
    pub async fn insert(
        &mut self,
        group_id: i64,
        connection: &NewGitProviderConnection,
    ) -> Result<StoredGitProviderConnection> {
        insert_git_provider_connection_on(&mut *self.tx, group_id, connection).await
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

/// Reads one connection through any executor, so the plain pool read and the
/// locked transaction read share one statement and one shape.
async fn get_git_provider_connection_on<'e, E>(
    executor: E,
    group_id: i64,
    connection_key: &str,
) -> Result<Option<StoredGitProviderConnection>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_file_as!(
        GitProviderConnectionRow,
        "src/sql/db/git_repositories/get_git_provider_connection.sql",
        group_id,
        connection_key
    )
    .fetch_optional(executor)
    .await?;
    row.map(StoredGitProviderConnection::from_row).transpose()
}

/// Runs the create-only insert through any executor, so the plain pool insert
/// and the locked transaction insert share one statement and one shape.
async fn insert_git_provider_connection_on<'e, E>(
    executor: E,
    group_id: i64,
    connection: &NewGitProviderConnection,
) -> Result<StoredGitProviderConnection>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_file_as!(
        GitProviderConnectionRow,
        "src/sql/db/git_repositories/insert_git_provider_connection.sql",
        group_id,
        connection.connection_key,
        connection.provider.as_str(),
        connection.mode.as_str(),
        connection.display_name,
        connection.base_url,
        connection.credential_secret_key,
        connection.webhook_secret_key
    )
    .fetch_one(executor)
    .await?;
    StoredGitProviderConnection::from_row(row)
}

#[cfg(test)]
mod secret_reference_tests {
    //! The two narrow connection statements against a disposable database.
    //!
    //! These drive [`crate::services::git_secrets::GitSecretWriter`] rather than
    //! calling the statements directly, because the property under test is the
    //! pair of them: a value is sealed under its own purpose *before* the
    //! reference moves, so a rotation that matches no row can only orphan a
    //! sealed row, never clear the reference already in place.

    use crate::{
        contracts::sources::{GitConnectionMode, GitProviderKind},
        db::{Database, NewGitProviderConnection, StoredGitProviderConnection},
        services::{
            git_secrets::{GitSecretSlot, GitSecretTarget, GitSecretWrite, GitSecretWriter},
            secret_store::{SecretPurpose, SecretStore, SecretStoreError},
        },
    };
    use sqlx::Row;
    use uuid::Uuid;

    const CONNECTION_KEY: &str = "github-app";
    /// Synthetic, and only ever compared against: no deployment credential.
    const TOKEN: &[u8] = b"token-bytes";
    const APP_KEY: &[u8] = b"pem-bytes";
    /// A group id that owns no connection at all, so an update matches no row.
    const UNOWNED_GROUP: i64 = 9_999_999;

    /// One group, one disabled connection, and a keyed store over the scratch
    /// pool. Skipped unless `CONTEXT69_TEST_DATABASE_URL` names a migrated
    /// disposable database; rows use a fresh UUID each run and never collide.
    async fn fixture() -> Option<(Database, i64, SecretStore)> {
        let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        let key = format!("git-secret-{}", Uuid::new_v4());
        let group_id: i64 = sqlx::query(
            "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
             VALUES ($1, 'Git Secret Test', 'private', 'shared', $2) RETURNING id",
        )
        .bind(&key)
        .bind(format!("/{key}"))
        .fetch_one(db.pool())
        .await
        .expect("seed group")
        .get("id");
        db.upsert_git_provider_connection(
            group_id,
            &NewGitProviderConnection {
                connection_key: CONNECTION_KEY.to_string(),
                provider: GitProviderKind::GitHub,
                mode: GitConnectionMode::Installation,
                display_name: "GitHub App".to_string(),
                base_url: "https://api.github.com".to_string(),
                credential_secret_key: None,
                webhook_secret_key: None,
            },
        )
        .await
        .expect("seed connection");
        assert!(
            db.disable_git_provider_connection(group_id, CONNECTION_KEY)
                .await
                .expect("disable connection")
        );
        // A per-run key, so nothing here is a credential of any deployment.
        let master_key = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            Uuid::new_v4().into_bytes().repeat(2),
        );
        let store = SecretStore::new(
            context69_secret_store::SecretDatabase::new(db.pool().clone()),
            Some(master_key.as_str()),
            1,
        )
        .expect("a store builds from any master key");
        Some((db, group_id, store))
    }

    fn target(group_id: i64) -> GitSecretTarget {
        GitSecretTarget::Connection {
            group_id,
            connection_key: CONNECTION_KEY.to_string(),
        }
    }

    async fn read(db: &Database, group_id: i64) -> StoredGitProviderConnection {
        db.get_git_provider_connection(group_id, CONNECTION_KEY)
            .await
            .expect("read the connection")
            .expect("the connection is still there")
    }

    async fn seal_and_point(
        writer: &GitSecretWriter,
        group_id: i64,
        slot: GitSecretSlot,
        value: &[u8],
    ) -> String {
        match writer
            .write(&target(group_id), slot, Some(value))
            .await
            .unwrap_or_else(|error| panic!("seal {slot:?}: {error}"))
        {
            GitSecretWrite::Stored(name) => name.as_str().to_string(),
            GitSecretWrite::Kept => panic!("a value is stored, not kept"),
        }
    }

    /// Both connection slots reach their own column under their own purpose, a
    /// disabled connection stays disabled, `None` and empty bytes keep what is
    /// there, and a no-match reference update leaves the owner's row untouched.
    #[tokio::test]
    async fn a_connection_rotation_moves_one_reference_and_preserves_disabled_state() {
        let Some((db, group_id, store)) = fixture().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the writer round trip");
            return;
        };
        let writer = GitSecretWriter::new(db.clone(), store.clone());
        let credential = seal_and_point(&writer, group_id, GitSecretSlot::Credential, TOKEN).await;
        let app_key =
            seal_and_point(&writer, group_id, GitSecretSlot::AppPrivateKey, APP_KEY).await;
        let row = read(&db, group_id).await;
        assert_eq!(
            row.credential_secret_key.as_deref(),
            Some(credential.as_str())
        );
        assert_eq!(
            row.app_private_key_secret_key.as_deref(),
            Some(app_key.as_str())
        );
        assert_ne!(credential, app_key, "each slot has its own key name");
        assert!(
            row.webhook_secret_key.is_none(),
            "a credential or App key never reaches the legacy webhook column"
        );
        assert!(
            row.disabled_at.is_some(),
            "a rotation must not re-enable a connection an operator disabled"
        );
        // Each value is sealed under its own purpose.
        for (purpose, key, value) in [
            (SecretPurpose::GitProviderToken, &credential, TOKEN),
            (SecretPurpose::GitHubAppPrivateKey, &app_key, APP_KEY),
        ] {
            assert_eq!(
                store
                    .get(purpose, key)
                    .await
                    .expect("open the sealed value")
                    .expect("it is stored")
                    .expose(),
                value,
                "the value is in the store immediately, under its own purpose"
            );
        }
        // Keep: an absent value and an empty one change neither side.
        for keep in [None, Some(&b""[..])] {
            assert_eq!(
                writer
                    .write(&target(group_id), GitSecretSlot::Credential, keep)
                    .await
                    .expect("keep"),
                GitSecretWrite::Kept,
                "an absent or empty value must not clear what is stored"
            );
        }
        assert_eq!(
            read(&db, group_id).await.credential_secret_key,
            Some(credential.clone())
        );
        assert_eq!(
            store
                .get(SecretPurpose::GitProviderToken, &credential)
                .await
                .expect("open the sealed value")
                .expect("it is stored")
                .expose(),
            TOKEN
        );

        // A write aimed at a group that owns no such record seals that group's own
        // key name and matches no row: the bounded failure is an orphan, never a
        // loss, and the owner's reference and value both survive.
        let error = writer
            .write(
                &target(UNOWNED_GROUP),
                GitSecretSlot::Credential,
                Some(b"orphan-bytes"),
            )
            .await
            .expect_err("no connection of that group is updated");
        let message = error.to_string();
        assert!(
            message.contains("git_provider.token") && message.contains("credential_secret_key"),
            "{message}"
        );
        for forbidden in ["orphan-bytes", "token-bytes", CONNECTION_KEY, &credential] {
            assert!(
                !message.contains(forbidden),
                "the failure carries no value or record key: {message}"
            );
        }
        let survivor = read(&db, group_id).await;
        assert_eq!(
            survivor.credential_secret_key.as_deref(),
            Some(credential.as_str()),
            "a failed reference update never clears the reference already in place"
        );
        assert!(
            survivor.disabled_at.is_some(),
            "and it never re-enables the connection"
        );

        // Last, because it re-seals a throwaway: a row whose stored purpose is the
        // App key is refused by a read of the same key name under the credential
        // purpose, so the two cannot reach each other's bytes even when the
        // names match. The frame authenticates the purpose as well as the name.
        store
            .write(SecretPurpose::GitProviderToken, &app_key, APP_KEY)
            .await
            .expect("seal App-key bytes under a credential read's key name");
        let crossed = store
            .get(SecretPurpose::GitHubAppPrivateKey, &app_key)
            .await;
        assert!(
            matches!(crossed, Err(SecretStoreError::PurposeMismatch)),
            "a value sealed for one purpose is refused, not answered, as another"
        );
    }

    /// The statements are conditional on the owning group, so a reference can
    /// only ever move inside the group that owns the connection.
    #[tokio::test]
    async fn a_connection_reference_move_is_group_confined() {
        let Some((db, group_id, _)) = fixture().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the confinement case");
            return;
        };
        for (owner, key) in [
            (UNOWNED_GROUP, CONNECTION_KEY),
            (group_id, "no-such-connection"),
        ] {
            assert!(
                !db.set_git_connection_credential_secret_key(owner, key, "k")
                    .await
                    .expect("run the statement"),
                "no row of another group and no unknown key may match: {owner}/{key}"
            );
        }
        assert!(
            !db.set_git_connection_app_private_key_secret_key(UNOWNED_GROUP, CONNECTION_KEY, "k")
                .await
                .expect("run the statement")
        );
        assert_eq!(
            read(&db, group_id).await.credential_secret_key,
            None,
            "a statement that matched nothing moved nothing"
        );
    }

    /// [`GitSecretWriter::seal`] writes the value and returns the reference it
    /// was stored under while moving no column, so a create path can obtain the
    /// reference before the row exists and insert the two together;
    /// [`GitSecretWriter::write`] is the only path that then repoints it.
    #[tokio::test]
    async fn a_seal_only_write_returns_the_reference_without_repointing() {
        let Some((db, group_id, store)) = fixture().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the seal-only round trip");
            return;
        };
        let writer = GitSecretWriter::new(db.clone(), store.clone());
        let sealed = match writer
            .seal(&target(group_id), GitSecretSlot::Credential, Some(TOKEN))
            .await
            .expect("seal the credential")
        {
            GitSecretWrite::Stored(name) => name.as_str().to_string(),
            GitSecretWrite::Kept => panic!("a value is stored, not kept"),
        };
        // The value is in the store under its own purpose immediately...
        assert!(
            store
                .get(SecretPurpose::GitProviderToken, &sealed)
                .await
                .expect("open the sealed value")
                .expect("it is stored")
                .expose()
                == TOKEN,
            "the sealed value round-trips under its own purpose"
        );
        // ...and no reference column moved, so the reference is available
        // before any row exists.
        let row = read(&db, group_id).await;
        assert!(
            row.credential_secret_key.is_none(),
            "seal alone must not repoint the credential"
        );
        assert!(row.app_private_key_secret_key.is_none());
        assert!(row.webhook_secret_key.is_none());

        // `write` is the operation that moves the reference, and it moves the
        // exact name `seal` returned.
        writer
            .write(&target(group_id), GitSecretSlot::Credential, Some(TOKEN))
            .await
            .expect("write the credential");
        assert!(
            read(&db, group_id).await.credential_secret_key.as_deref() == Some(sealed.as_str()),
            "write moves the exact reference seal returned"
        );
        // Keep stays a no-op for the seal-only path too.
        for value in [None, Some(&b""[..])] {
            assert_eq!(
                writer
                    .seal(&target(group_id), GitSecretSlot::Credential, value)
                    .await
                    .expect("keep"),
                GitSecretWrite::Kept,
                "an absent or empty value must not seal anything"
            );
        }
    }
}
