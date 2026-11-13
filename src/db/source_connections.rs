//! Persistence for source connections.
//!
//! The row carries two things a caller must not confuse: the `name`, which is a
//! user-controlled API/display identifier, and `connection_key`, the stable UUID
//! the sealed database URL is keyed by in the shared secret store. The row also
//! records the `internal_secrets` key that owns the value. Reads project those
//! three, so a consumer can answer presence from metadata and resolve the value
//! through the store without this layer knowing anything about encryption — and
//! without the DSN ever being written here.

use anyhow::Result;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ActiveValue::Set, EntityTrait, QueryOrder};

use super::entity::runtime_source_connections;
use super::{Database, NewSourceConnection, SourceConnectionRow, StoredSourceConnection};

impl From<runtime_source_connections::Model> for StoredSourceConnection {
    fn from(row: runtime_source_connections::Model) -> Self {
        Self {
            name: row.name,
            connection_key: row.connection_key,
            database_url_secret_key: row.database_url_secret_key,
        }
    }
}

impl From<SourceConnectionRow> for StoredSourceConnection {
    fn from(row: SourceConnectionRow) -> Self {
        Self {
            name: row.name,
            connection_key: row.connection_key,
            database_url_secret_key: row.database_url_secret_key,
        }
    }
}

fn connection_upsert(
    connection: &NewSourceConnection,
) -> sea_orm::Insert<runtime_source_connections::ActiveModel> {
    runtime_source_connections::Entity::insert(runtime_source_connections::ActiveModel {
        name: Set(connection.name.clone()),
        connection_key: Set(connection.connection_key),
        database_url_secret_key: Set(Some(connection.database_url_secret_key.clone())),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(runtime_source_connections::Column::Name)
            .update_column(runtime_source_connections::Column::DatabaseUrlSecretKey)
            .value(
                runtime_source_connections::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .to_owned(),
    )
}

fn connection_database_error(error: sea_orm::DbErr) -> anyhow::Error {
    use sea_orm::{DbErr, RuntimeErr};
    let (error, wrap): (_, fn(RuntimeErr) -> DbErr) = match error {
        DbErr::Exec(RuntimeErr::SqlxError(error)) => (error, DbErr::Exec),
        DbErr::Query(RuntimeErr::SqlxError(error)) => (error, DbErr::Query),
        DbErr::Conn(RuntimeErr::SqlxError(error)) => (error, DbErr::Conn),
        error => return error.into(),
    };
    match std::sync::Arc::try_unwrap(error) {
        Ok(error) => error.into(),
        Err(error) => wrap(RuntimeErr::SqlxError(error)).into(),
    }
}

impl Database {
    /// Every stored connection, without opening any sealed value.
    pub async fn list_source_connections(&self) -> Result<Vec<StoredSourceConnection>> {
        let rows = runtime_source_connections::Entity::find()
            .order_by_asc(runtime_source_connections::Column::Name)
            .all(&self.orm)
            .await
            .map_err(connection_database_error)?;

        Ok(rows.into_iter().map(StoredSourceConnection::from).collect())
    }

    /// One connection by its user-controlled name, or `None` when unknown.
    pub async fn get_source_connection(
        &self,
        name: &str,
    ) -> Result<Option<StoredSourceConnection>> {
        let row = runtime_source_connections::Entity::find_by_id(name.to_owned())
            .one(&self.orm)
            .await
            .map_err(connection_database_error)?;

        Ok(row.map(StoredSourceConnection::from))
    }

    /// Creates or updates one connection under a known stable identity.
    ///
    /// The caller seals the database URL under
    /// `connection_key.database_url_secret_key` before calling this, because the
    /// reference has to point at a store row that already exists. The identity
    /// itself is never rewritten: an update keeps the `connection_key` the row
    /// was created with, so a re-save cannot orphan the sealed value.
    pub async fn save_source_connection(
        &self,
        connection: &NewSourceConnection,
    ) -> Result<StoredSourceConnection> {
        let row = connection_upsert(connection)
            .exec_with_returning(&self.orm)
            .await
            .map_err(connection_database_error)?;

        Ok(StoredSourceConnection::from(row))
    }

    /// The registry row of one source connection, on a borrowed connection.
    ///
    /// A task-owned write saves the row on the connection of its guarded
    /// transaction, so the registry row and the grant that authorized it commit
    /// together. Sharing a registry row with another task does not authorize a
    /// stale task to write it.
    pub async fn save_source_connection_in(
        &self,
        connection: &mut sqlx::PgConnection,
        source: &NewSourceConnection,
    ) -> Result<StoredSourceConnection> {
        let row = sqlx::query_file_as!(
            SourceConnectionRow,
            "src/sql/db/source_connections/save_source_connection.sql",
            source.connection_key,
            source.name,
            source.database_url_secret_key
        )
        .fetch_one(&mut *connection)
        .await?;

        Ok(StoredSourceConnection::from(row))
    }

    /// Removes the connection record, reporting whether one existed.
    ///
    /// Clearing the connection's sealed database URL is the caller's step and
    /// happens before this call, so a store row is never orphaned by a delete
    /// that then fails.
    pub async fn delete_source_connection(&self, name: &str) -> Result<bool> {
        let result = runtime_source_connections::Entity::delete_by_id(name.to_owned())
            .exec(&self.orm)
            .await
            .map_err(connection_database_error)?;
        Ok(result.rows_affected > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::{SourceConnectionRow, connection_upsert};
    use crate::db::entity::runtime_source_connections;
    use crate::db::{NewSourceConnection, StoredSourceConnection};
    use sea_orm::{DbBackend, EntityTrait, QueryOrder, QueryTrait};
    use uuid::Uuid;

    const GET_MIGRATION_SQL: &str =
        include_str!("../../migrations/20261002010450_source_connection_secret_key.sql");
    const DROP_MIGRATION_SQL: &str =
        include_str!("../../migrations/20261002094933_drop_legacy_reversible_secret_columns.sql");
    const SAVE_SQL: &str = include_str!("../sql/db/source_connections/save_source_connection.sql");

    fn row() -> SourceConnectionRow {
        SourceConnectionRow {
            name: "primary".to_string(),
            connection_key: Uuid::new_v4(),
            database_url_secret_key: Some("source_connection.database_url.key".to_string()),
        }
    }

    #[test]
    fn the_identity_migration_is_additive_and_keeps_the_legacy_column() {
        assert!(
            GET_MIGRATION_SQL
                .contains("ADD COLUMN connection_key UUID NOT NULL DEFAULT gen_random_uuid()"),
            "each connection gets a stable identity this application mints"
        );
        assert!(
            GET_MIGRATION_SQL.contains("ADD COLUMN database_url_secret_key TEXT")
                && GET_MIGRATION_SQL
                    .contains("REFERENCES context69.internal_secrets(key) ON DELETE SET NULL"),
            "the sealed value is referenced from the store, optionally"
        );
        assert!(
            GET_MIGRATION_SQL.contains("UNIQUE (connection_key)"),
            "the identity is unique so it can identify a connection"
        );

        let lower = GET_MIGRATION_SQL.to_ascii_lowercase();
        for forbidden in [
            "drop column",
            "update context69.runtime_source_connections set database_url",
            "delete from",
            "truncate",
        ] {
            assert!(
                !lower.contains(forbidden),
                "the migration must not rewrite or clear a legacy value: found {forbidden}"
            );
        }
    }

    /// The cleanup migration removes the plaintext column and the constraint that
    /// required it, and nothing else about the connection survives it.
    #[test]
    fn the_cleanup_migration_drops_the_plaintext_column_and_its_check() {
        assert!(
            DROP_MIGRATION_SQL.contains(
                "ALTER TABLE context69.runtime_source_connections\n    DROP CONSTRAINT \
                 runtime_source_connections_database_url_check,\n    DROP COLUMN database_url;"
            ),
            "the derived non-blank check goes with the column it guarded: {DROP_MIGRATION_SQL}"
        );
        // Only the statements are inspected: the header comments are allowed to name
        // what the migration deliberately leaves alone.
        let statements = DROP_MIGRATION_SQL
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();
        for forbidden in [
            "update ",
            "insert ",
            "delete from",
            "translation_provider_settings",
            "connection_key",
            "database_url_secret_key",
        ] {
            assert!(
                !statements.contains(forbidden),
                "the cleanup migration only drops columns: found {forbidden}"
            );
        }
    }

    #[test]
    fn every_read_projects_the_identity_and_the_store_reference() {
        let query = runtime_source_connections::Entity::find()
            .order_by_asc(runtime_source_connections::Column::Name)
            .build(DbBackend::Postgres)
            .to_string();
        assert!(query.contains("\"connection_key\""), "{query}");
        assert!(query.contains("\"database_url_secret_key\""), "{query}");
        assert!(!query.contains("\"database_url\""), "{query}");
        assert!(
            query.contains("ORDER BY \"runtime_source_connections\".\"name\" ASC"),
            "{query}"
        );
    }

    #[test]
    fn saving_never_moves_a_connection_onto_a_new_identity_or_writes_the_dsn() {
        assert!(
            SAVE_SQL.contains("ON CONFLICT (name) DO UPDATE")
                && !SAVE_SQL.contains("connection_key = EXCLUDED.connection_key"),
            "an update must keep the identity the connection was created with"
        );
        assert!(
            SAVE_SQL.contains("RETURNING name, connection_key, database_url_secret_key"),
            "the saved row reports the identity that is now in effect"
        );
        assert!(
            SAVE_SQL.contains("database_url_secret_key = EXCLUDED.database_url_secret_key"),
            "the store reference is what the save writes"
        );
        let lower = SAVE_SQL.to_ascii_lowercase();
        assert!(
            !lower.contains("database_url =") && !lower.contains("database_url,"),
            "the plaintext DSN is never written by the connection row: {SAVE_SQL}"
        );
    }

    #[test]
    fn native_upsert_preserves_stable_identity() {
        let connection = NewSourceConnection {
            name: "primary".into(),
            connection_key: Uuid::nil(),
            database_url_secret_key: "secret".into(),
        };
        let query = connection_upsert(&connection)
            .build(DbBackend::Postgres)
            .to_string();
        let update = query
            .split("DO UPDATE SET")
            .nth(1)
            .expect("conflict update");
        assert!(update.contains("\"database_url_secret_key\""), "{query}");
        assert!(!update.contains("\"connection_key\""), "{query}");
        assert!(!query.contains("\"database_url\""), "{query}");
    }

    #[test]
    fn deleting_a_connection_removes_only_its_own_record() {
        let query = runtime_source_connections::Entity::delete_by_id("primary".to_string())
            .build(DbBackend::Postgres)
            .to_string();
        assert!(
            query.contains("DELETE FROM \"context69\".\"runtime_source_connections\""),
            "{query}"
        );
        assert!(query.contains("\"name\" = 'primary'"), "{query}");
        assert!(!query.contains("internal_secrets"), "{query}");
    }

    #[test]
    fn a_row_maps_onto_the_stored_connection_without_inventing_an_identity() {
        let row = row();
        let connection: Uuid = row.connection_key;
        let stored = StoredSourceConnection::from(row);

        assert_eq!(stored.name, "primary");
        assert_eq!(stored.connection_key, connection);
        assert_eq!(
            stored.database_url_secret_key.as_deref(),
            Some("source_connection.database_url.key")
        );
    }
}
