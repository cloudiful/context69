use anyhow::Result;

use super::{Database, SourceConnectionRow, StoredSourceConnection};

impl Database {
    pub async fn list_source_connections(&self) -> Result<Vec<StoredSourceConnection>> {
        let rows = sqlx::query_as::<_, SourceConnectionRow>(
            r#"
            SELECT name, database_url
            FROM context69.runtime_source_connections
            ORDER BY name
            "#,
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
        let row = sqlx::query_as::<_, SourceConnectionRow>(
            r#"
            SELECT name, database_url
            FROM context69.runtime_source_connections
            WHERE name = $1
            "#,
        )
        .bind(name)
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
        let row = sqlx::query_as::<_, SourceConnectionRow>(
            r#"
            INSERT INTO context69.runtime_source_connections (
                name,
                database_url,
                updated_at
            )
            VALUES ($1, $2, now())
            ON CONFLICT (name) DO UPDATE
            SET database_url = EXCLUDED.database_url,
                updated_at = now()
            RETURNING name, database_url
            "#,
        )
        .bind(&connection.name)
        .bind(&connection.database_url)
        .fetch_one(&self.pool)
        .await?;

        Ok(StoredSourceConnection {
            name: row.name,
            database_url: row.database_url,
        })
    }

    pub async fn delete_source_connection(&self, name: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM context69.runtime_source_connections
            WHERE name = $1
            "#,
        )
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
