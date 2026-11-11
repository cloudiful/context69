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
