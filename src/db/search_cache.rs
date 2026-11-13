use std::collections::HashMap;

use anyhow::{Context, Result};
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict};
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::entity::{rerank_item_scores as scores, search_generations as generations};
use super::{Database, StoredRerankItemScore};

impl Database {
    pub async fn get_search_generation(&self) -> Result<i64> {
        let row = generations::Entity::find_by_id("global")
            .one(&self.orm)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        Ok(row.generation)
    }

    pub async fn bump_search_generation(&self) -> Result<i64> {
        let row = generations::Entity::insert(generations::ActiveModel {
            scope: Set("global".to_owned()),
            generation: Set(1),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(generations::Column::Scope)
                .value(
                    generations::Column::Generation,
                    Expr::col((generations::Entity, generations::Column::Generation)).add(1),
                )
                .value(generations::Column::UpdatedAt, Expr::current_timestamp())
                .to_owned(),
        )
        .exec_with_returning(&self.orm)
        .await?;
        Ok(row.generation)
    }

    /// The search generation, advanced on a borrowed connection.
    ///
    /// A task-owned increment runs on the connection of its guarded
    /// transaction, so the counter and the authority that advanced it commit
    /// together.
    pub async fn bump_search_generation_in(
        &self,
        connection: &mut sqlx::PgConnection,
    ) -> Result<i64> {
        let generation =
            sqlx::query_file_scalar!("src/sql/db/search_cache/bump_search_generation.sql")
                .fetch_one(connection)
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
        let rows = scores::Entity::update_many()
            .col_expr(scores::Column::LastUsedAt, Expr::current_timestamp())
            .filter(scores::Column::RerankModel.eq(rerank_model))
            .filter(scores::Column::QueryHash.eq(query_hash))
            .filter(Expr::cust_with_values(
                "chunk_id = ANY($1)",
                [sea_orm::Value::from(chunk_ids.to_vec())],
            ))
            .exec_with_returning(&self.orm)
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
        let result = scores::Entity::delete_many()
            .filter(Expr::cust_with_values(
                "last_used_at < now() - make_interval(days => $1::int)",
                [retention_days],
            ))
            .exec(&self.orm)
            .await?;
        Ok(result.rows_affected)
    }
}
