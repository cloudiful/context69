use anyhow::{Context, Result};

use super::Database;

impl Database {
    pub async fn get_vector_index_fingerprint(
        &self,
        collection_name: &str,
    ) -> Result<Option<String>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/vector_index_state/get.sql", collection_name)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn save_vector_index_state(&self, state: &VectorIndexState<'_>) -> Result<()> {
        let dimensions =
            i64::try_from(state.dimensions).context("embedding dimensions too large")?;
        let rebuilt_chunks =
            i64::try_from(state.rebuilt_chunks).context("rebuilt chunk count too large")?;
        sqlx::query_file!(
            "src/sql/db/vector_index_state/save.sql",
            state.collection_name,
            state.fingerprint,
            state.embedding_base_url,
            state.embedding_model,
            dimensions,
            rebuilt_chunks
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

pub struct VectorIndexState<'a> {
    pub collection_name: &'a str,
    pub fingerprint: &'a str,
    pub embedding_base_url: &'a str,
    pub embedding_model: &'a str,
    pub dimensions: usize,
    pub rebuilt_chunks: usize,
}
