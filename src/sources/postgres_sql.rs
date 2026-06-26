use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row};

use crate::{
    config::{PostgresSqlConnectorConfig, SyncStrategy},
    domain::{SourceRecord, SyncCheckpoint},
    sources::SourceConnector,
};

#[derive(Clone)]
pub struct PostgresSqlSourceConnector {
    pool: PgPool,
    source_key: String,
    sync_strategy: SyncStrategy,
    config: PostgresSqlConnectorConfig,
}

impl PostgresSqlSourceConnector {
    pub fn new(
        pool: PgPool,
        source_key: String,
        sync_strategy: SyncStrategy,
        config: PostgresSqlConnectorConfig,
    ) -> Self {
        Self {
            pool,
            source_key,
            sync_strategy,
            config,
        }
    }

    fn wrapped_query(&self, checkpoint: &SyncCheckpoint) -> String {
        let base = format!(
            "SELECT external_id, title, body_text, source_uri, updated_at, metadata_json, summary, published_at FROM ({}) AS context69_source",
            self.config.base_query
        );

        match self.sync_strategy {
            SyncStrategy::Cursor | SyncStrategy::FullScan => {
                if checkpoint.updated_at.is_some() {
                    format!(
                        "{base} WHERE (updated_at > $1) OR (updated_at = $1 AND external_id > $2) ORDER BY updated_at ASC, external_id ASC LIMIT $3"
                    )
                } else {
                    format!("{base} ORDER BY updated_at ASC, external_id ASC LIMIT $1")
                }
            }
        }
    }
}

#[async_trait]
impl SourceConnector for PostgresSqlSourceConnector {
    async fn validate(&self) -> Result<()> {
        let query = format!(
            "SELECT * FROM ({}) AS context69_source LIMIT 1",
            self.config.base_query
        );
        let maybe_row = sqlx::query(AssertSqlSafe(query))
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = maybe_row {
            let _: String = row.try_get("external_id")?;
            let _: String = row.try_get("title")?;
            let _: String = row.try_get("body_text")?;
            let _: String = row.try_get("source_uri")?;
            let _: DateTime<Utc> = row.try_get("updated_at")?;
            let _metadata: Option<Value> = row.try_get("metadata_json")?;
        }

        Ok(())
    }

    async fn fetch_batch(&self, checkpoint: &SyncCheckpoint) -> Result<Vec<SourceRecord>> {
        let query = self.wrapped_query(checkpoint);
        let rows = if let Some(updated_at) = checkpoint.updated_at {
            sqlx::query(AssertSqlSafe(query))
                .bind(updated_at)
                .bind(checkpoint.external_id.as_deref().unwrap_or_default())
                .bind(self.config.batch_size)
                .fetch_all(&self.pool)
                .await
                .with_context(|| format!("failed to fetch batch for source {}", self.source_key))?
        } else {
            sqlx::query(AssertSqlSafe(query))
                .bind(self.config.batch_size)
                .fetch_all(&self.pool)
                .await
                .with_context(|| format!("failed to fetch batch for source {}", self.source_key))?
        };

        rows.into_iter()
            .map(|row| {
                Ok(SourceRecord {
                    external_id: row.try_get("external_id")?,
                    title: row.try_get("title")?,
                    body_text: row.try_get("body_text")?,
                    source_uri: row.try_get("source_uri")?,
                    summary: row.try_get("summary").ok(),
                    published_at: row
                        .try_get::<Option<NaiveDate>, _>("published_at")
                        .ok()
                        .flatten(),
                    updated_at: row.try_get("updated_at")?,
                    metadata_json: row
                        .try_get::<Option<Value>, _>("metadata_json")?
                        .unwrap_or_else(|| serde_json::json!({})),
                })
            })
            .collect()
    }
}
