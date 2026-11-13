//! Group-scoped provider connection persistence (issue #681 work unit 3B1).
//!
//! Every operation takes the owning group: reads filter on it, and upserts
//! conflict on `(group_id, connection_key)`, so one group can neither read nor
//! take over another group's connection.

use anyhow::Result;
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use sqlx::{Postgres, Transaction};

use crate::db::entity::git_provider_connections::{Column, Entity};

use super::rows::GitProviderConnectionRow;
use super::types::{NewGitProviderConnection, StoredGitProviderConnection};
use crate::db::Database;

pub(super) fn list_connections_query(
    group_id: i64,
) -> sea_orm::SelectTwo<Entity, crate::db::entity::groups::Entity> {
    use crate::db::entity::groups;
    Entity::find()
        .find_also_related(groups::Entity)
        .filter(Column::GroupId.eq(group_id))
        .filter(groups::Column::Id.is_not_null())
        .order_by_asc(Column::ConnectionKey)
}

pub(super) fn connection_secret_query(
    group_id: i64,
    connection_key: &str,
    secret_key: &str,
    column: Column,
) -> sea_orm::UpdateMany<Entity> {
    Entity::update_many()
        .col_expr(column, Expr::value(secret_key))
        .col_expr(Column::UpdatedAt, Expr::current_timestamp())
        .filter(Column::GroupId.eq(group_id))
        .filter(Column::ConnectionKey.eq(connection_key))
}

pub(super) fn connection_lifecycle_query(
    group_id: i64,
    connection_key: &str,
    enabled: bool,
) -> sea_orm::UpdateMany<Entity> {
    let disabled = if enabled {
        Expr::value(None::<chrono::DateTime<chrono::Utc>>)
    } else {
        Func::coalesce([Expr::col(Column::DisabledAt), Expr::current_timestamp()]).into()
    };
    Entity::update_many()
        .col_expr(Column::DisabledAt, disabled)
        .col_expr(Column::UpdatedAt, Expr::current_timestamp())
        .filter(Column::GroupId.eq(group_id))
        .filter(Column::ConnectionKey.eq(connection_key))
}

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
        let rows = list_connections_query(group_id).all(&self.orm).await?;
        rows.into_iter()
            .filter_map(|(row, group)| {
                group.map(|group| GitProviderConnectionRow {
                    group_id: row.group_id,
                    group_key: group.group_key,
                    group_path: group.full_path,
                    group_visibility: group.visibility,
                    connection_key: row.connection_key,
                    provider_kind: row.provider_kind,
                    connection_mode: row.connection_mode,
                    display_name: row.display_name,
                    base_url: row.base_url,
                    credential_secret_key: row.credential_secret_key,
                    webhook_secret_key: row.webhook_secret_key,
                    app_private_key_secret_key: row.app_private_key_secret_key,
                    disabled_at: row.disabled_at,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                })
            })
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
        let result = connection_secret_query(
            group_id,
            connection_key,
            secret_key,
            Column::CredentialSecretKey,
        )
        .exec(&self.orm)
        .await?;
        Ok(result.rows_affected > 0)
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
        let result = connection_secret_query(
            group_id,
            connection_key,
            secret_key,
            Column::AppPrivateKeySecretKey,
        )
        .exec(&self.orm)
        .await?;
        Ok(result.rows_affected > 0)
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
        let result = connection_lifecycle_query(group_id, connection_key, true)
            .exec(&self.orm)
            .await?;
        Ok(result.rows_affected > 0)
    }

    /// Disables a connection owned by `group_id`, reporting whether a row
    /// matched. A connection of another group never matches.
    pub async fn disable_git_provider_connection(
        &self,
        group_id: i64,
        connection_key: &str,
    ) -> Result<bool> {
        let result = connection_lifecycle_query(group_id, connection_key, false)
            .exec(&self.orm)
            .await?;
        Ok(result.rows_affected > 0)
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
