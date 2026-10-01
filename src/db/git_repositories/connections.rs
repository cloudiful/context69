//! Group-scoped provider connection persistence (issue #681 work unit 3B1).
//!
//! Every operation takes the owning group: reads filter on it, and upserts
//! conflict on `(group_id, connection_key)`, so one group can neither read nor
//! take over another group's connection.

use anyhow::Result;

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

    /// Reads one connection owned by `group_id`; a connection of another group
    /// reads as absent.
    pub async fn get_git_provider_connection(
        &self,
        group_id: i64,
        connection_key: &str,
    ) -> Result<Option<StoredGitProviderConnection>> {
        let row = sqlx::query_file_as!(
            GitProviderConnectionRow,
            "src/sql/db/git_repositories/get_git_provider_connection.sql",
            group_id,
            connection_key
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitProviderConnection::from_row).transpose()
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
