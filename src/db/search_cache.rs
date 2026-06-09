use std::collections::HashMap;

use anyhow::{Context, Result};
use uuid::Uuid;

use super::{Database, RerankItemScoreRow, StoredRerankItemScore};

impl Database {
    pub async fn get_search_generation(&self) -> Result<i64> {
        let generation = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT generation
            FROM context69.search_generations
            WHERE scope = 'global'
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(generation)
    }

    pub async fn bump_search_generation(&self) -> Result<i64> {
        let generation = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO context69.search_generations (scope, generation, updated_at)
            VALUES ('global', 1, now())
            ON CONFLICT (scope) DO UPDATE
            SET generation = context69.search_generations.generation + 1,
                updated_at = now()
            RETURNING generation
            "#,
        )
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

        let rows = sqlx::query_as::<_, RerankItemScoreRow>(
            r#"
            UPDATE context69.rerank_item_scores
            SET last_used_at = now()
            WHERE rerank_model = $1
              AND query_hash = $2
              AND chunk_id = ANY($3)
            RETURNING rerank_model, query_hash, query_text_trimmed, chunk_id, score
            "#,
        )
        .bind(rerank_model)
        .bind(query_hash)
        .bind(chunk_ids)
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
            .map(|score| score.rerank_model.as_str())
            .collect::<Vec<_>>();
        let query_hashes = scores
            .iter()
            .map(|score| score.query_hash.as_str())
            .collect::<Vec<_>>();
        let query_texts = scores
            .iter()
            .map(|score| score.query_text_trimmed.as_str())
            .collect::<Vec<_>>();
        let chunk_ids = scores.iter().map(|score| score.chunk_id).collect::<Vec<_>>();
        let values = scores.iter().map(|score| score.score).collect::<Vec<_>>();

        sqlx::query(
            r#"
            INSERT INTO context69.rerank_item_scores (
                rerank_model,
                query_hash,
                query_text_trimmed,
                chunk_id,
                score,
                created_at,
                last_used_at
            )
            SELECT
                item.rerank_model,
                item.query_hash,
                item.query_text_trimmed,
                item.chunk_id,
                item.score,
                now(),
                now()
            FROM unnest(
                $1::text[],
                $2::text[],
                $3::text[],
                $4::uuid[],
                $5::real[]
            ) AS item(rerank_model, query_hash, query_text_trimmed, chunk_id, score)
            ON CONFLICT (rerank_model, query_hash, chunk_id) DO UPDATE
            SET score = EXCLUDED.score,
                query_text_trimmed = EXCLUDED.query_text_trimmed,
                last_used_at = now()
            "#,
        )
        .bind(&rerank_models)
        .bind(&query_hashes)
        .bind(&query_texts)
        .bind(&chunk_ids)
        .bind(&values)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete_expired_rerank_item_scores(&self, retention_days: i64) -> Result<u64> {
        let deleted = sqlx::query(
            r#"
            DELETE FROM context69.rerank_item_scores
            WHERE last_used_at < now() - make_interval(days => $1::int)
            "#,
        )
        .bind(i32::try_from(retention_days).context("retention_days out of range")?)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(deleted)
    }
}
