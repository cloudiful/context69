use anyhow::Result;

use super::Database;

impl Database {
    pub async fn get_or_create_internal_secret(
        &self,
        key: &str,
        candidate: &[u8],
    ) -> Result<Vec<u8>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/internal_secrets/get_or_create_internal_secret.sql",
            key,
            candidate
        )
        .fetch_one(&self.pool)
        .await?)
    }
}
