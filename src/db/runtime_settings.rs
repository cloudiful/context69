use anyhow::{Context, Result};

use crate::{domain_errors::DomainError, vector_runtime::VectorIdentityAck};

use super::{
    Database, RegistryQuotaPolicy, RuntimeChunkingSettingsRow, RuntimeEmbeddingSettingsRow,
    RuntimeFileLibrarySettingsRow, RuntimeQdrantSettingsRow, RuntimeSchedulerSettingsRow,
    StoredRuntimeChunkingSettings, StoredRuntimeEmbeddingSettings,
    StoredRuntimeFileLibrarySettings, StoredRuntimeQdrantSettings, StoredRuntimeS3Settings,
    StoredRuntimeSchedulerSettings, StoredRuntimeSettings,
};
use crate::contracts::settings::SchedulerResourceLimits;

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
        // The registry capacity policy is read from the row that holds it, so
        // the reported policy is the one the admission enforces.
        let quota = self.read_registry_quota().await?;

        Ok(Some(StoredRuntimeSettings {
            qdrant: StoredRuntimeQdrantSettings {
                url: qdrant.url,
                collection_name: qdrant.collection_name,
                recreate_on_dimension_mismatch: qdrant.recreate_on_dimension_mismatch,
            },
            embedding: StoredRuntimeEmbeddingSettings {
                base_url: embedding.base_url,
                api_key: None,
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
                resource_limits: SchedulerResourceLimits {
                    interactive: scheduler
                        .interactive_max_concurrency
                        .map(|value| {
                            u32::try_from(value)
                                .context("runtime scheduler interactive limit must be non-negative")
                        })
                        .transpose()?,
                    bulk: scheduler
                        .bulk_max_concurrency
                        .map(|value| {
                            u32::try_from(value)
                                .context("runtime scheduler bulk limit must be non-negative")
                        })
                        .transpose()?,
                    maintenance: scheduler
                        .maintenance_max_concurrency
                        .map(|value| {
                            u32::try_from(value)
                                .context("runtime scheduler maintenance limit must be non-negative")
                        })
                        .transpose()?,
                    ingest: scheduler
                        .ingest_max_concurrency
                        .map(|value| {
                            u32::try_from(value)
                                .context("runtime scheduler ingest limit must be non-negative")
                        })
                        .transpose()?,
                },
                ingest_batches_per_segment: validated_ingest_batches_per_segment(
                    scheduler.ingest_batches_per_segment,
                )?,
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
                url_import_concurrency: usize::try_from(file_library.url_import_concurrency)
                    .context("runtime file_library url_import_concurrency must be non-negative")?,
                url_import_min_interval_ms: u64::try_from(file_library.url_import_min_interval_ms)
                    .context(
                        "runtime file_library url_import_min_interval_ms must be non-negative",
                    )?,
                trusted_proxy_enabled: file_library.trusted_proxy_enabled,
                registry_max_ids_total: Some(quota.max_ids_total),
                registry_max_ids_per_file: Some(quota.max_ids_per_file),
                registry_max_ids_per_run: Some(quota.max_ids_per_run),
                // The S3 block is projected from the identifiers alone. Its secret
                // key is not in this row, so it is resolved from the shared store by
                // whoever needs a usable configuration.
                s3: match (
                    file_library.s3_endpoint,
                    file_library.s3_region,
                    file_library.s3_bucket,
                    file_library.s3_access_key,
                ) {
                    (Some(endpoint), Some(region), Some(bucket), Some(access_key)) => {
                        Some(StoredRuntimeS3Settings {
                            endpoint,
                            region,
                            bucket,
                            prefix: file_library.s3_prefix.unwrap_or_default(),
                            path_style: file_library.s3_path_style,
                            access_key,
                            secret_key: None,
                        })
                    }
                    _ => None,
                },
            },
        }))
    }

    /// The configured number of ingest batches one task indexing segment
    /// commits.
    ///
    /// An absent singleton row uses the application default. A persisted value
    /// outside the accepted range fails closed instead of being clamped.
    pub async fn get_ingest_batches_per_segment(&self) -> Result<usize> {
        let value: Option<i32> = sqlx::query_file_scalar!(
            "src/sql/db/runtime_settings/get_ingest_batches_per_segment.sql"
        )
        .fetch_optional(&self.pool)
        .await?;
        match value {
            Some(value) => validated_ingest_batches_per_segment(i64::from(value)),
            None => Ok(crate::contracts::settings::INGEST_BATCHES_PER_SEGMENT_DEFAULT),
        }
    }

    pub async fn save_runtime_settings(
        &self,
        settings: &StoredRuntimeSettings,
    ) -> Result<StoredRuntimeSettings> {
        self.save_runtime_settings_with_identity_ack(settings, None)
            .await
    }

    /// Saves the runtime settings family and, when `acknowledgment` is present,
    /// rewrites the persisted vector identity in the same transaction.
    ///
    /// The acknowledgment is conditioned on the exact collection, the fingerprint
    /// currently recorded, and its dimensions. A missing or mismatched row fails
    /// the whole save, so settings never commit while the stored vectors still
    /// claim a different identity. The update touches only the fingerprint,
    /// endpoint, model, and dimensions, preserving `rebuilt_at` and
    /// `rebuilt_chunks`.
    pub async fn save_runtime_settings_with_identity_ack(
        &self,
        settings: &StoredRuntimeSettings,
        acknowledgment: Option<VectorIdentityAck<'_>>,
    ) -> Result<StoredRuntimeSettings> {
        let mut tx = self.pool.begin().await?;
        // The rebuild barrier is checked under the epoch lock inside this
        // transaction, so a rebuild that starts concurrently serializes with
        // this save instead of racing it. The lock order is epoch, runtime
        // settings rows, vector identity row, which is the order every other
        // writer of these rows takes.
        self.lock_search_registry_epoch_in(&mut tx).await?;
        // The registry capacity policy is resolved under the same epoch lock
        // the admission takes, so a save and an admission describe one
        // policy. A request that does not send a cap keeps the stored one, and
        // an invalid policy is refused before any row is written.
        let registry_quota = self
            .resolve_registry_quota_in(&mut tx, &settings.file_library)
            .await?;
        self.require_quiet_vector_identity(&mut tx, settings)
            .await?;
        // The global and per-resource admission policies are validated at the
        // database boundary too, so a direct caller cannot persist a cap above
        // the global maximum.
        super::validate_execution_admission_policies(
            settings.scheduler.max_concurrency,
            &settings.scheduler.resource_limits,
        )?;
        let ingest_batches_per_segment = i32::try_from(validated_ingest_batches_per_segment(
            i64::try_from(settings.scheduler.ingest_batches_per_segment)
                .context("scheduler ingest_batches_per_segment too large")?,
        )?)
        .context("scheduler ingest_batches_per_segment too large")?;
        let embedding_dimensions = i64::try_from(settings.embedding.dimensions)
            .context("embedding dimensions too large")?;
        let embedding_timeout_secs = i64::try_from(settings.embedding.timeout_secs)
            .context("embedding timeout too large")?;
        let scheduler_interval_secs = i64::try_from(settings.scheduler.interval_secs)
            .context("scheduler interval too large")?;
        let scheduler_max_concurrency = i64::try_from(settings.scheduler.max_concurrency)
            .context("scheduler max_concurrency too large")?;
        let scheduler_limits = &settings.scheduler.resource_limits;
        let scheduler_interactive_limit = scheduler_limits
            .interactive
            .map(i32::try_from)
            .transpose()
            .context("interactive concurrency limit too large")?;
        let scheduler_bulk_limit = scheduler_limits
            .bulk
            .map(i32::try_from)
            .transpose()
            .context("bulk concurrency limit too large")?;
        let scheduler_maintenance_limit = scheduler_limits
            .maintenance
            .map(i32::try_from)
            .transpose()
            .context("maintenance concurrency limit too large")?;
        let scheduler_ingest_limit = scheduler_limits
            .ingest
            .map(i32::try_from)
            .transpose()
            .context("ingest concurrency limit too large")?;
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
        let file_library_url_import_concurrency =
            i64::try_from(settings.file_library.url_import_concurrency)
                .context("file_library url_import_concurrency too large")?;
        let file_library_url_import_min_interval_ms =
            i64::try_from(settings.file_library.url_import_min_interval_ms)
                .context("file_library url_import_min_interval_ms too large")?;

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
            settings.scheduler.valkey_url,
            scheduler_interactive_limit,
            scheduler_bulk_limit,
            scheduler_maintenance_limit,
            scheduler_ingest_limit,
            ingest_batches_per_segment
        )
        .execute(&mut *tx)
        .await?;

        // The settings columns and their live policies commit together: the
        // library writes both the global cap and every owned resource key
        // through this same transaction.
        self.apply_execution_admission_policies_in(
            &mut tx,
            settings.scheduler.max_concurrency,
            &settings.scheduler.resource_limits,
        )
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
            file_library_url_import_concurrency,
            file_library_url_import_min_interval_ms,
            settings.file_library.trusted_proxy_enabled,
            settings
                .file_library
                .s3
                .as_ref()
                .map(|value| value.endpoint.as_str()),
            settings
                .file_library
                .s3
                .as_ref()
                .map(|value| value.region.as_str()),
            settings
                .file_library
                .s3
                .as_ref()
                .map(|value| value.bucket.as_str()),
            settings
                .file_library
                .s3
                .as_ref()
                .map(|value| value.prefix.as_str()),
            settings
                .file_library
                .s3
                .as_ref()
                .is_some_and(|value| value.path_style),
            settings
                .file_library
                .s3
                .as_ref()
                .map(|value| value.access_key.as_str()),
            registry_quota.max_ids_total,
            registry_quota.max_ids_per_file,
            registry_quota.max_ids_per_run
        )
        .execute(&mut *tx)
        .await?;

        if let Some(acknowledgment) = acknowledgment {
            let acknowledged_dimensions = i64::try_from(acknowledgment.expected_dimensions)
                .context("embedding dimensions too large")?;
            let affected = sqlx::query_file!(
                "src/sql/db/vector_index_state/acknowledge_identity.sql",
                acknowledgment.candidate_fingerprint,
                acknowledgment.embedding_base_url,
                acknowledgment.embedding_model,
                embedding_dimensions,
                acknowledgment.collection_name,
                acknowledgment.expected_fingerprint,
                acknowledged_dimensions
            )
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if affected != 1 {
                return Err(DomainError::conflict(
                    "the stored vector identity changed concurrently; the runtime settings were \
                     not saved",
                )
                .into());
            }
        }

        tx.commit().await?;
        self.get_runtime_settings()
            .await?
            .context("runtime settings missing after save")
    }

    /// Resolves the registry capacity policy of a settings save.
    ///
    /// A request that carries a cap replaces the stored one; a request that
    /// carries none keeps it. The stored values are read inside the save
    /// transaction, under the registry epoch lock, so a save and a concurrent
    /// admission describe the same policy. The resolved policy is validated
    /// before any row is written, so an invalid policy is refused atomically
    /// and never replaces a usable one.
    async fn resolve_registry_quota_in(
        &self,
        connection: &mut sqlx::PgConnection,
        file_library: &StoredRuntimeFileLibrarySettings,
    ) -> Result<RegistryQuotaPolicy> {
        let stored = self.read_usable_registry_quota_in(connection).await?;
        let policy = RegistryQuotaPolicy {
            max_ids_total: file_library
                .registry_max_ids_total
                .unwrap_or(stored.max_ids_total),
            max_ids_per_file: file_library
                .registry_max_ids_per_file
                .unwrap_or(stored.max_ids_per_file),
            max_ids_per_run: file_library
                .registry_max_ids_per_run
                .unwrap_or(stored.max_ids_per_run),
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Refuses a save that moves the vector space while a rebuild owns it.
    ///
    /// The decision is made against the persisted rows inside the save
    /// transaction, never against a service-level read that a concurrent
    /// rebuild could already have overtaken. A save that does not move the
    /// vector space - a credential, a timeout, an unrelated setting - is never
    /// blocked by a running rebuild or by an unresolved run.
    async fn require_quiet_vector_identity(
        &self,
        connection: &mut sqlx::PgConnection,
        settings: &StoredRuntimeSettings,
    ) -> Result<()> {
        let Some(persisted) = self.read_persisted_vector_identity_in(connection).await? else {
            // Nothing is configured yet, so there is no vector space a rebuild
            // could own.
            return Ok(());
        };
        let candidate = crate::db::RegistryVectorIdentity {
            embedding_fingerprint: crate::embedding::embedding_runtime::vector_identity_fingerprint(
                settings.qdrant.url.trim(),
                settings.qdrant.collection_name.trim(),
                settings.embedding.base_url.trim(),
                settings.embedding.model.trim(),
                settings.embedding.dimensions,
            ),
            collection_name: settings.qdrant.collection_name.trim().to_string(),
        };
        if candidate == persisted {
            return Ok(());
        }
        let barrier = self.read_rebuild_barrier_in(connection).await?;
        if barrier.state != crate::db::RebuildBarrierState::Idle {
            return Err(DomainError::conflict(
                "the runtime vector identity cannot change while a vector index rebuild is in \
                 progress; retry after it completes",
            )
            .into());
        }
        let staging = sqlx::query_file_scalar!("src/sql/db/search_registry/read_staging_count.sql")
            .fetch_one(&mut *connection)
            .await
            .context("read the unresolved staging entries")?;
        if staging > 0 {
            return Err(DomainError::conflict(
                "unresolved ingestion runs still hold staged points; retry the vector identity \
                 change after they resolved",
            )
            .into());
        }
        Ok(())
    }
}

/// Validates the persisted or saved ingest batch quantum.
///
/// A value outside the accepted range fails closed; it is never clamped.
fn validated_ingest_batches_per_segment(value: i64) -> Result<usize> {
    let value = usize::try_from(value).map_err(|_| {
        DomainError::invalid_argument(format!(
            "runtime.scheduler.ingest_batches_per_segment {value} must be between {} and {}",
            crate::contracts::settings::INGEST_BATCHES_PER_SEGMENT_MIN,
            crate::contracts::settings::INGEST_BATCHES_PER_SEGMENT_MAX
        ))
    })?;
    if !(crate::contracts::settings::INGEST_BATCHES_PER_SEGMENT_MIN
        ..=crate::contracts::settings::INGEST_BATCHES_PER_SEGMENT_MAX)
        .contains(&value)
    {
        return Err(DomainError::invalid_argument(format!(
            "runtime.scheduler.ingest_batches_per_segment {value} must be between {} and {}",
            crate::contracts::settings::INGEST_BATCHES_PER_SEGMENT_MIN,
            crate::contracts::settings::INGEST_BATCHES_PER_SEGMENT_MAX
        ))
        .into());
    }
    Ok(value)
}
