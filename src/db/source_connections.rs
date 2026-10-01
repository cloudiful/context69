//! Persistence for source connections.
//!
//! The row carries two things a caller must not confuse: the `name`, which is a
//! user-controlled API/display identifier, and `connection_key`, the stable UUID
//! the sealed database URL is keyed by in the shared secret store. Reads project
//! both plus the `internal_secrets` reference, so a consumer can answer presence
//! from metadata and resolve the value through the store without this layer
//! knowing anything about encryption.

use anyhow::Result;

use super::{Database, NewSourceConnection, SourceConnectionRow, StoredSourceConnection};

impl From<SourceConnectionRow> for StoredSourceConnection {
    fn from(row: SourceConnectionRow) -> Self {
        Self {
            name: row.name,
            connection_key: row.connection_key,
            database_url: row.database_url,
            database_url_secret_key: row.database_url_secret_key,
        }
    }
}

impl Database {
    /// Every stored connection, without opening any sealed value.
    pub async fn list_source_connections(&self) -> Result<Vec<StoredSourceConnection>> {
        let rows = sqlx::query_file_as!(
            SourceConnectionRow,
            "src/sql/db/source_connections/list_source_connections.sql"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(StoredSourceConnection::from).collect())
    }

    /// One connection by its user-controlled name, or `None` when unknown.
    pub async fn get_source_connection(
        &self,
        name: &str,
    ) -> Result<Option<StoredSourceConnection>> {
        let row = sqlx::query_file_as!(
            SourceConnectionRow,
            "src/sql/db/source_connections/get_source_connection.sql",
            name
        )
        .fetch_optional(&self.pool)
        .await?;

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
        let row = sqlx::query_file_as!(
            SourceConnectionRow,
            "src/sql/db/source_connections/save_source_connection.sql",
            connection.connection_key,
            connection.name,
            connection.database_url,
            connection.database_url_secret_key
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(StoredSourceConnection::from(row))
    }

    /// Removes the legacy record, reporting whether a connection existed.
    ///
    /// Clearing the connection's sealed database URL is the caller's step and
    /// happens before this call, so a store row is never orphaned by a delete
    /// that then fails.
    pub async fn delete_source_connection(&self, name: &str) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/source_connections/delete_source_connection.sql",
            name
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::SourceConnectionRow;
    use crate::db::{Database, NewSourceConnection, StoredSourceConnection};
    use uuid::Uuid;

    const MIGRATION_SQL: &str =
        include_str!("../../migrations/20261002010450_source_connection_secret_key.sql");
    const GET_SQL: &str = include_str!("../sql/db/source_connections/get_source_connection.sql");
    const LIST_SQL: &str = include_str!("../sql/db/source_connections/list_source_connections.sql");
    const SAVE_SQL: &str = include_str!("../sql/db/source_connections/save_source_connection.sql");
    const DELETE_SQL: &str =
        include_str!("../sql/db/source_connections/delete_source_connection.sql");

    fn row() -> SourceConnectionRow {
        SourceConnectionRow {
            name: "primary".to_string(),
            connection_key: Uuid::new_v4(),
            database_url: "postgres://user:pass@host/db".to_string(),
            database_url_secret_key: Some("source_connection.database_url.key".to_string()),
        }
    }

    #[test]
    fn the_migration_is_additive_and_keeps_the_legacy_column() {
        assert!(
            MIGRATION_SQL
                .contains("ADD COLUMN connection_key UUID NOT NULL DEFAULT gen_random_uuid()"),
            "each connection gets a stable identity this application mints"
        );
        assert!(
            MIGRATION_SQL.contains("ADD COLUMN database_url_secret_key TEXT")
                && MIGRATION_SQL
                    .contains("REFERENCES context69.internal_secrets(key) ON DELETE SET NULL"),
            "the sealed value is referenced from the store, optionally"
        );
        assert!(
            MIGRATION_SQL.contains("UNIQUE (connection_key)"),
            "the identity is unique so it can identify a connection"
        );

        let lower = MIGRATION_SQL.to_ascii_lowercase();
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
        // The legacy column keeps its own column definition; nothing here
        // reinterprets or re-encodes what is already stored.
        assert!(
            !lower.contains("alter column database_url"),
            "the legacy database_url column must not be re-typed or re-constrained"
        );
    }

    #[test]
    fn every_read_projects_the_identity_and_the_store_reference() {
        for query in [GET_SQL, LIST_SQL] {
            assert!(
                query.contains("name, connection_key, database_url, database_url_secret_key"),
                "a read must project the name, the stable identity, the legacy value, and the \
                 store reference: {query}"
            );
        }
        assert!(
            LIST_SQL.contains("ORDER BY name"),
            "listing stays ordered by the user-facing identifier"
        );
    }

    #[test]
    fn saving_never_moves_a_connection_onto_a_new_identity() {
        assert!(
            SAVE_SQL.contains("ON CONFLICT (name) DO UPDATE")
                && !SAVE_SQL.contains("connection_key = EXCLUDED.connection_key"),
            "an update must keep the identity the connection was created with"
        );
        assert!(
            SAVE_SQL
                .contains("RETURNING name, connection_key, database_url, database_url_secret_key"),
            "the saved row reports the identity that is now in effect"
        );
        assert!(
            SAVE_SQL.contains("database_url = EXCLUDED.database_url")
                && SAVE_SQL.contains("database_url_secret_key = EXCLUDED.database_url_secret_key"),
            "the legacy column and the store reference are both written for the transition"
        );
    }

    #[test]
    fn deleting_a_connection_removes_only_the_legacy_record() {
        assert!(
            DELETE_SQL.contains("DELETE FROM context69.runtime_source_connections")
                && DELETE_SQL.contains("WHERE name = $1"),
            "the statement removes one named connection and nothing else: {DELETE_SQL}"
        );
        let lower = DELETE_SQL.to_ascii_lowercase();
        assert!(
            !lower.contains("internal_secrets"),
            "the store row is cleared by the owner, not by this statement"
        );
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

    /// The migration round trip, against a migrated scratch database.
    ///
    /// Skipped unless `CONTEXT69_TEST_DATABASE_URL` names one. Every row it
    /// writes is keyed by a fresh UUID and removed again, so it leaves nothing
    /// behind.
    #[tokio::test]
    async fn a_connection_round_trips_its_identity_and_store_reference() {
        let Ok(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL") else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the round trip");
            return;
        };
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        let name = format!("source-connection-round-trip-{}", Uuid::new_v4());
        let connection_key = Uuid::new_v4();
        let secret_key = format!("source_connection.database_url.{connection_key}");

        // The reference resolves to a store row, which is why the seal happens
        // before the save.
        sqlx::query("INSERT INTO context69.internal_secrets (key, value) VALUES ($1, $2)")
            .bind(&secret_key)
            .bind(b"sealed-bytes".as_slice())
            .execute(db.pool())
            .await
            .expect("seed the store row the reference points at");

        let saved = db
            .save_source_connection(&NewSourceConnection {
                connection_key,
                name: name.clone(),
                database_url: "postgres://one/db".to_string(),
                database_url_secret_key: secret_key.clone(),
            })
            .await
            .expect("insert the connection");
        assert_eq!(saved.connection_key, connection_key);

        // A re-save that supplies a different identity must not move the
        // connection onto it, or the sealed value would be orphaned.
        let resaved = db
            .save_source_connection(&NewSourceConnection {
                connection_key: Uuid::new_v4(),
                name: name.clone(),
                database_url: "postgres://two/db".to_string(),
                database_url_secret_key: secret_key.clone(),
            })
            .await
            .expect("update the connection");
        assert_eq!(
            resaved.connection_key, connection_key,
            "an update keeps the identity the connection was created with"
        );
        assert_eq!(resaved.database_url, "postgres://two/db");

        let fetched = db
            .get_source_connection(&name)
            .await
            .expect("read the connection")
            .expect("the connection exists");
        assert_eq!(fetched.connection_key, connection_key);
        assert_eq!(
            fetched.database_url_secret_key.as_deref(),
            Some(secret_key.as_str())
        );
        assert!(
            db.list_source_connections()
                .await
                .expect("list connections")
                .iter()
                .any(|connection| connection.name == name
                    && connection.connection_key == connection_key),
            "a listed connection carries the same identity and reference"
        );

        assert!(db.delete_source_connection(&name).await.expect("delete"));
        assert!(
            !db.delete_source_connection(&name)
                .await
                .expect("delete again"),
            "deleting an unknown connection reports that nothing was removed"
        );
        sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
            .bind(&secret_key)
            .execute(db.pool())
            .await
            .expect("remove the seeded store row");
    }
}
