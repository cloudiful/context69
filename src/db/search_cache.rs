use std::collections::HashMap;

use anyhow::{Context, Result};
use uuid::Uuid;

use super::{Database, RerankItemScoreRow, StoredRerankItemScore};

impl Database {
    pub async fn get_search_generation(&self) -> Result<i64> {
        let generation =
            sqlx::query_file_scalar!("src/sql/db/search_cache/get_search_generation.sql")
                .fetch_one(&self.pool)
                .await?;
        Ok(generation)
    }

    pub async fn bump_search_generation(&self) -> Result<i64> {
        let generation =
            sqlx::query_file_scalar!("src/sql/db/search_cache/bump_search_generation.sql")
                .fetch_one(&self.pool)
                .await?;
        Ok(generation)
    }

    pub async fn list_rerank_item_scores(
        &self,
        rerank_model: &str,
        query_hash: &str,
        chunk_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, StoredRerankItemScore>> {
        if chunk_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = sqlx::query_file_as!(
            RerankItemScoreRow,
            "src/sql/db/search_cache/list_rerank_item_scores.sql",
            rerank_model,
            query_hash,
            chunk_ids
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.chunk_id,
                    StoredRerankItemScore {
                        rerank_model: row.rerank_model,
                        query_hash: row.query_hash,
                        query_text_trimmed: row.query_text_trimmed,
                        chunk_id: row.chunk_id,
                        score: row.score,
                    },
                )
            })
            .collect())
    }

    pub async fn upsert_rerank_item_scores(&self, scores: &[StoredRerankItemScore]) -> Result<()> {
        if scores.is_empty() {
            return Ok(());
        }

        let rerank_models = scores
            .iter()
            .map(|score| score.rerank_model.clone())
            .collect::<Vec<_>>();
        let query_hashes = scores
            .iter()
            .map(|score| score.query_hash.clone())
            .collect::<Vec<_>>();
        let query_texts = scores
            .iter()
            .map(|score| score.query_text_trimmed.clone())
            .collect::<Vec<_>>();
        let chunk_ids = scores
            .iter()
            .map(|score| score.chunk_id)
            .collect::<Vec<_>>();
        let values = scores.iter().map(|score| score.score).collect::<Vec<_>>();

        sqlx::query_file!(
            "src/sql/db/search_cache/upsert_rerank_item_scores.sql",
            &rerank_models,
            &query_hashes,
            &query_texts,
            &chunk_ids,
            &values
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete_expired_rerank_item_scores(&self, retention_days: i64) -> Result<u64> {
        let retention_days =
            i32::try_from(retention_days).context("retention_days out of range")?;
        let deleted = sqlx::query_file!(
            "src/sql/db/search_cache/delete_expired_rerank_item_scores.sql",
            retention_days
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(deleted)
    }
}
