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
        Ok(
            sqlx::query_file_scalar!(
                "src/sql/db/runtime_settings/runtime_settings_initialized.sql"
            )
            .fetch_one(&self.pool)
            .await?,
        )
    }

    pub async fn get_runtime_settings(&self) -> Result<Option<StoredRuntimeSettings>> {
        let qdrant = sqlx::query_file_as!(
            RuntimeQdrantSettingsRow,
            "src/sql/db/runtime_settings/get_runtime_qdrant_settings.sql"
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(qdrant) = qdrant else {
            return Ok(None);
        };

        let embedding = sqlx::query_file_as!(
            RuntimeEmbeddingSettingsRow,
            "src/sql/db/runtime_settings/get_runtime_embedding_settings.sql"
        )
        .fetch_one(&self.pool)
        .await?;
        let scheduler = sqlx::query_file_as!(
            RuntimeSchedulerSettingsRow,
            "src/sql/db/runtime_settings/get_runtime_scheduler_settings.sql"
        )
        .fetch_one(&self.pool)
        .await?;
        let chunking = sqlx::query_file_as!(
            RuntimeChunkingSettingsRow,
            "src/sql/db/runtime_settings/get_runtime_chunking_settings.sql"
        )
        .fetch_one(&self.pool)
        .await?;
        let file_library = sqlx::query_file_as!(
            RuntimeFileLibrarySettingsRow,
            "src/sql/db/runtime_settings/get_runtime_file_library_settings.sql"
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
                base_url: embedding.base_url,
                api_key: embedding.api_key,
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
        let embedding_dimensions = i64::try_from(settings.embedding.dimensions)
            .context("embedding dimensions too large")?;
        let embedding_timeout_secs = i64::try_from(settings.embedding.timeout_secs)
            .context("embedding timeout too large")?;
        let scheduler_interval_secs = i64::try_from(settings.scheduler.interval_secs)
            .context("scheduler interval too large")?;
        let scheduler_max_concurrency = i64::try_from(settings.scheduler.max_concurrency)
            .context("scheduler max_concurrency too large")?;
        let chunking_max_chars =
            i64::try_from(settings.chunking.max_chars).context("chunking max_chars too large")?;
        let chunking_overlap_chars = i64::try_from(settings.chunking.overlap_chars)
            .context("chunking overlap_chars too large")?;
        let file_library_max_upload_size_mb =
            i64::try_from(settings.file_library.max_upload_size_mb)
                .context("file_library max_upload_size_mb too large")?;
        let file_library_max_upload_request_size_mb =
            i64::try_from(settings.file_library.max_upload_request_size_mb)
                .context("file_library max_upload_request_size_mb too large")?;
        let file_library_ingest_concurrency =
            i64::try_from(settings.file_library.ingest_concurrency)
                .context("file_library ingest_concurrency too large")?;

        sqlx::query_file!(
            "src/sql/db/runtime_settings/save_runtime_qdrant_settings.sql",
            settings.qdrant.url,
            settings.qdrant.collection_name,
            settings.qdrant.recreate_on_dimension_mismatch
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query_file!(
            "src/sql/db/runtime_settings/save_runtime_embedding_settings.sql",
            settings.embedding.base_url,
            settings.embedding.api_key,
            settings.embedding.model,
            embedding_dimensions,
            embedding_timeout_secs
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query_file!(
            "src/sql/db/runtime_settings/save_runtime_scheduler_settings.sql",
            scheduler_interval_secs,
            settings.scheduler.run_on_start,
            scheduler_max_concurrency,
            settings.scheduler.job_id,
            settings.scheduler.valkey_url
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query_file!(
            "src/sql/db/runtime_settings/save_runtime_chunking_settings.sql",
            chunking_max_chars,
            chunking_overlap_chars
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query_file!(
            "src/sql/db/runtime_settings/save_runtime_file_library_settings.sql",
            settings.file_library.storage_root,
            file_library_max_upload_size_mb,
            file_library_max_upload_request_size_mb,
            file_library_ingest_concurrency,
            i64::from(settings.file_library.pdf_pages_per_task)
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        self.get_runtime_settings()
            .await?
            .context("runtime settings missing after save")
    }
}
