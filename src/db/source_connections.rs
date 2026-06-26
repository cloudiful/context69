use anyhow::Result;

use super::{Database, SourceConnectionRow, StoredSourceConnection};

impl Database {
    pub async fn list_source_connections(&self) -> Result<Vec<StoredSourceConnection>> {
        let rows = sqlx::query_file_as!(
            SourceConnectionRow,
            "src/sql/db/source_connections/list_source_connections.sql"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| StoredSourceConnection {
                name: row.name,
                database_url: row.database_url,
            })
            .collect())
    }

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

        Ok(row.map(|row| StoredSourceConnection {
            name: row.name,
            database_url: row.database_url,
        }))
    }

    pub async fn save_source_connection(
        &self,
        connection: &StoredSourceConnection,
    ) -> Result<StoredSourceConnection> {
        let row = sqlx::query_file_as!(
            SourceConnectionRow,
            "src/sql/db/source_connections/save_source_connection.sql",
            connection.name,
            connection.database_url
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(StoredSourceConnection {
            name: row.name,
            database_url: row.database_url,
        })
    }

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
