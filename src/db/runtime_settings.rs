use anyhow::{Context, Result};

use super::{
    Database, RuntimeChunkingSettingsRow, RuntimeEmbeddingSettingsRow,
    RuntimeFileLibrarySettingsRow, RuntimeQdrantSettingsRow, RuntimeSchedulerSettingsRow,
    StoredRuntimeChunkingSettings, StoredRuntimeEmbeddingSettings,
    StoredRuntimeFileLibrarySettings, StoredRuntimeQdrantSettings, StoredRuntimeSchedulerSettings,
    StoredRuntimeSettings,
};

impl Database {
    pub async fn runtime_settings_initialized(&self) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM context69.runtime_qdrant_settings
                WHERE singleton = TRUE
            )
            "#,
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_runtime_settings(&self) -> Result<Option<StoredRuntimeSettings>> {
        let qdrant = sqlx::query_as::<_, RuntimeQdrantSettingsRow>(
            r#"
            SELECT url, collection_name, recreate_on_dimension_mismatch
            FROM context69.runtime_qdrant_settings
            WHERE singleton = TRUE
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(qdrant) = qdrant else {
            return Ok(None);
        };

        let embedding = sqlx::query_as::<_, RuntimeEmbeddingSettingsRow>(
            r#"
            SELECT provider_account_key, model, dimensions, timeout_secs
            FROM context69.runtime_embedding_settings
            WHERE singleton = TRUE
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        let scheduler = sqlx::query_as::<_, RuntimeSchedulerSettingsRow>(
            r#"
            SELECT interval_secs, run_on_start, max_concurrency, job_id, valkey_url
            FROM context69.runtime_scheduler_settings
            WHERE singleton = TRUE
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        let chunking = sqlx::query_as::<_, RuntimeChunkingSettingsRow>(
            r#"
            SELECT max_chars, overlap_chars
            FROM context69.runtime_chunking_settings
            WHERE singleton = TRUE
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        let file_library = sqlx::query_as::<_, RuntimeFileLibrarySettingsRow>(
            r#"
            SELECT
                storage_root,
                max_upload_size_mb,
                max_upload_request_size_mb,
                ingest_concurrency,
                pdf_pages_per_task
            FROM context69.runtime_file_library_settings
            WHERE singleton = TRUE
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Some(StoredRuntimeSettings {
            qdrant: StoredRuntimeQdrantSettings {
                url: qdrant.url,
                collection_name: qdrant.collection_name,
                recreate_on_dimension_mismatch: qdrant.recreate_on_dimension_mismatch,
            },
            embedding: StoredRuntimeEmbeddingSettings {
                provider_account_key: embedding.provider_account_key,
                model: embedding.model,
                dimensions: usize::try_from(embedding.dimensions)
                    .context("runtime embedding dimensions must be non-negative")?,
                timeout_secs: u64::try_from(embedding.timeout_secs)
                    .context("runtime embedding timeout_secs must be non-negative")?,
            },
            scheduler: StoredRuntimeSchedulerSettings {
                interval_secs: u64::try_from(scheduler.interval_secs)
                    .context("runtime scheduler interval_secs must be non-negative")?,
                run_on_start: scheduler.run_on_start,
                max_concurrency: usize::try_from(scheduler.max_concurrency)
                    .context("runtime scheduler max_concurrency must be non-negative")?,
                job_id: scheduler.job_id,
                valkey_url: scheduler.valkey_url,
            },
            chunking: StoredRuntimeChunkingSettings {
                max_chars: usize::try_from(chunking.max_chars)
                    .context("runtime chunking max_chars must be non-negative")?,
                overlap_chars: usize::try_from(chunking.overlap_chars)
                    .context("runtime chunking overlap_chars must be non-negative")?,
            },
            file_library: StoredRuntimeFileLibrarySettings {
                storage_root: file_library.storage_root,
                max_upload_size_mb: usize::try_from(file_library.max_upload_size_mb)
                    .context("runtime file_library max_upload_size_mb must be non-negative")?,
                max_upload_request_size_mb: usize::try_from(
                    file_library.max_upload_request_size_mb,
                )
                .context("runtime file_library max_upload_request_size_mb must be non-negative")?,
                ingest_concurrency: usize::try_from(file_library.ingest_concurrency)
                    .context("runtime file_library ingest_concurrency must be non-negative")?,
                pdf_pages_per_task: u32::try_from(file_library.pdf_pages_per_task)
                    .context("runtime file_library pdf_pages_per_task must be non-negative")?,
            },
        }))
    }

    pub async fn save_runtime_settings(
        &self,
        settings: &StoredRuntimeSettings,
    ) -> Result<StoredRuntimeSettings> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            r#"
            INSERT INTO context69.runtime_qdrant_settings (
                singleton,
                url,
                collection_name,
                recreate_on_dimension_mismatch,
                updated_at
            )
            VALUES (TRUE, $1, $2, $3, now())
            ON CONFLICT (singleton) DO UPDATE
            SET url = EXCLUDED.url,
                collection_name = EXCLUDED.collection_name,
                recreate_on_dimension_mismatch = EXCLUDED.recreate_on_dimension_mismatch,
                updated_at = now()
            "#,
        )
        .bind(&settings.qdrant.url)
        .bind(&settings.qdrant.collection_name)
        .bind(settings.qdrant.recreate_on_dimension_mismatch)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO context69.runtime_embedding_settings (
                singleton,
                provider_account_key,
                model,
                dimensions,
                timeout_secs,
                updated_at
            )
            VALUES (TRUE, $1, $2, $3, $4, now())
            ON CONFLICT (singleton) DO UPDATE
            SET provider_account_key = EXCLUDED.provider_account_key,
                model = EXCLUDED.model,
                dimensions = EXCLUDED.dimensions,
                timeout_secs = EXCLUDED.timeout_secs,
                updated_at = now()
            "#,
        )
        .bind(&settings.embedding.provider_account_key)
        .bind(&settings.embedding.model)
        .bind(
            i64::try_from(settings.embedding.dimensions)
                .context("embedding dimensions too large")?,
        )
        .bind(
            i64::try_from(settings.embedding.timeout_secs)
                .context("embedding timeout too large")?,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO context69.runtime_scheduler_settings (
                singleton,
                interval_secs,
                run_on_start,
                max_concurrency,
                job_id,
                valkey_url,
                updated_at
            )
            VALUES (TRUE, $1, $2, $3, $4, $5, now())
            ON CONFLICT (singleton) DO UPDATE
            SET interval_secs = EXCLUDED.interval_secs,
                run_on_start = EXCLUDED.run_on_start,
                max_concurrency = EXCLUDED.max_concurrency,
                job_id = EXCLUDED.job_id,
                valkey_url = EXCLUDED.valkey_url,
                updated_at = now()
            "#,
        )
        .bind(
            i64::try_from(settings.scheduler.interval_secs)
                .context("scheduler interval too large")?,
        )
        .bind(settings.scheduler.run_on_start)
        .bind(
            i64::try_from(settings.scheduler.max_concurrency)
                .context("scheduler max_concurrency too large")?,
        )
        .bind(&settings.scheduler.job_id)
        .bind(&settings.scheduler.valkey_url)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO context69.runtime_chunking_settings (
                singleton,
                max_chars,
                overlap_chars,
                updated_at
            )
            VALUES (TRUE, $1, $2, now())
            ON CONFLICT (singleton) DO UPDATE
            SET max_chars = EXCLUDED.max_chars,
                overlap_chars = EXCLUDED.overlap_chars,
                updated_at = now()
            "#,
        )
        .bind(i64::try_from(settings.chunking.max_chars).context("chunking max_chars too large")?)
        .bind(
            i64::try_from(settings.chunking.overlap_chars)
                .context("chunking overlap_chars too large")?,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO context69.runtime_file_library_settings (
                singleton,
                storage_root,
                max_upload_size_mb,
                max_upload_request_size_mb,
                ingest_concurrency,
                pdf_pages_per_task,
                updated_at
            )
            VALUES (TRUE, $1, $2, $3, $4, $5, now())
            ON CONFLICT (singleton) DO UPDATE
            SET storage_root = EXCLUDED.storage_root,
                max_upload_size_mb = EXCLUDED.max_upload_size_mb,
                max_upload_request_size_mb = EXCLUDED.max_upload_request_size_mb,
                ingest_concurrency = EXCLUDED.ingest_concurrency,
                pdf_pages_per_task = EXCLUDED.pdf_pages_per_task,
                updated_at = now()
            "#,
        )
        .bind(&settings.file_library.storage_root)
        .bind(
            i64::try_from(settings.file_library.max_upload_size_mb)
                .context("file_library max_upload_size_mb too large")?,
        )
        .bind(
            i64::try_from(settings.file_library.max_upload_request_size_mb)
                .context("file_library max_upload_request_size_mb too large")?,
        )
        .bind(
            i64::try_from(settings.file_library.ingest_concurrency)
                .context("file_library ingest_concurrency too large")?,
        )
        .bind(i64::from(settings.file_library.pdf_pages_per_task))
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        self.get_runtime_settings()
            .await?
            .context("runtime settings missing after save")
    }
}
