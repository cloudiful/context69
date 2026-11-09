use anyhow::{Context, Result};

use crate::chunking::ChunkBudgetMode;
use crate::chunking::chunking_token::TokenizerRef;
use crate::contracts::settings::EmbeddingBatchingSettings;
use crate::{domain_errors::DomainError, vector_runtime::VectorIdentityAck};

use super::{
    Database, RegistryQuotaPolicy, RuntimeChunkingSettingsRow, RuntimeEmbeddingSettingsRow,
    RuntimeFileLibrarySettingsRow, RuntimeQdrantSettingsRow, RuntimeSchedulerSettingsRow,
    StoredRuntimeChunkingSettings, StoredRuntimeCodeEmbeddingSettings,
    StoredRuntimeEmbeddingSettings, StoredRuntimeFileLibrarySettings, StoredRuntimeQdrantSettings,
    StoredRuntimeS3Settings, StoredRuntimeSchedulerSettings, StoredRuntimeSettings,
};

/// The persisted code embedding settings row.
///
/// The row lives here rather than in the shared row module because every field
/// except `enabled` is nullable for an explicitly disabled configuration.
#[derive(Debug, Clone, sqlx::FromRow)]
struct RuntimeCodeEmbeddingSettingsRow {
    enabled: bool,
    base_url: Option<String>,
    model: Option<String>,
    query_model: Option<String>,
    dimensions: Option<i64>,
    timeout_secs: Option<i64>,
    max_items_per_request: Option<i64>,
    max_item_chars: Option<i64>,
    max_request_chars: Option<i64>,
    max_request_bytes: Option<i64>,
}

/// The persisted name of a chunking unit.
fn chunking_unit_value(unit: &ChunkBudgetMode) -> &'static str {
    match unit {
        ChunkBudgetMode::Chars => "chars",
        ChunkBudgetMode::Tokens => "tokens",
    }
}

/// The chunking unit a persisted row names.
///
/// A row that names a unit this build does not know is refused instead of
/// being read as the character budget, so a chunker never runs with a budget
/// the operator did not select.
fn stored_chunking_unit(value: &str) -> Result<ChunkBudgetMode> {
    match value {
        "chars" => Ok(ChunkBudgetMode::Chars),
        "tokens" => Ok(ChunkBudgetMode::Tokens),
        other => Err(anyhow::anyhow!(
            "the persisted chunking unit {other:?} is not supported"
        )),
    }
}

/// The tokenizer a persisted row names.
fn stored_chunking_tokenizer(value: &str, sha256: Option<String>) -> Result<TokenizerRef> {
    match value {
        "hf_local" => {
            let sha256 = sha256.ok_or_else(|| {
                anyhow::anyhow!("the persisted hf_local chunking tokenizer carries no sha256")
            })?;
            let reference = TokenizerRef::HfLocal { sha256 };
            reference.validate().map_err(anyhow::Error::from)?;
            Ok(reference)
        }
        other => {
            let kind = crate::chunking::chunking_token::TokenizerKind::from_name(other)
                .ok_or_else(|| {
                    anyhow::anyhow!("the persisted chunking tokenizer {other:?} is not supported")
                })?;
            if sha256.is_some() {
                anyhow::bail!(
                    "the persisted chunking tokenizer {other:?} carries an artifact hash"
                );
            }
            Ok(kind.into())
        }
    }
}

/// The batching limits a persisted row describes.
///
/// A row that predates the limits stores NULL for them, which is the published
/// default: a deployment that never configured a limit keeps the documented
/// one instead of losing its planner.
fn stored_embedding_batching(
    row: &RuntimeEmbeddingSettingsRow,
) -> Result<EmbeddingBatchingSettings> {
    let defaults = EmbeddingBatchingSettings::default();
    Ok(EmbeddingBatchingSettings {
        max_items_per_request: stored_batching_limit(
            row.max_items_per_request,
            defaults.max_items_per_request,
        )?,
        max_item_chars: stored_batching_limit(row.max_item_chars, defaults.max_item_chars)?,
        max_request_chars: stored_batching_limit(
            row.max_request_chars,
            defaults.max_request_chars,
        )?,
        max_request_bytes: stored_batching_limit(
            row.max_request_bytes,
            defaults.max_request_bytes,
        )?,
    })
}

/// One persisted batching limit, or the default when the row carries none.
fn stored_batching_limit(value: Option<i64>, fallback: usize) -> Result<usize> {
    match value {
        None => Ok(fallback),
        Some(value) => {
            usize::try_from(value).context("runtime embedding batching limit must be non-negative")
        }
    }
}

fn optional_positive_usize(value: Option<i64>, label: &str) -> Result<usize> {
    match value {
        None => Ok(0),
        Some(value) => {
            usize::try_from(value).with_context(|| format!("{label} must be non-negative"))
        }
    }
}

fn optional_positive_u64(value: Option<i64>, label: &str) -> Result<u64> {
    match value {
        None => Ok(0),
        Some(value) => {
            u64::try_from(value).with_context(|| format!("{label} must be non-negative"))
        }
    }
}

fn optional_text(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn optional_i64(value: usize, label: &str) -> Result<Option<i64>> {
    if value == 0 {
        return Ok(None);
    }
    Ok(Some(
        i64::try_from(value).with_context(|| format!("{label} too large"))?,
    ))
}

fn optional_i64_u64(value: u64, label: &str) -> Result<Option<i64>> {
    if value == 0 {
        return Ok(None);
    }
    Ok(Some(
        i64::try_from(value).with_context(|| format!("{label} too large"))?,
    ))
}

async fn write_runtime_code_embedding_settings_in(
    connection: &mut sqlx::PgConnection,
    code: &StoredRuntimeCodeEmbeddingSettings,
) -> Result<()> {
    let dimensions = optional_i64(code.dimensions, "code embedding dimensions")?;
    let timeout_secs = optional_i64_u64(code.timeout_secs, "code embedding timeout_secs")?;
    sqlx::query_file!(
        "src/sql/db/runtime_settings/save_runtime_code_embedding_settings.sql",
        code.enabled,
        optional_text(&code.base_url),
        optional_text(&code.model),
        code.query_model.as_deref(),
        dimensions,
        timeout_secs,
        i64::try_from(code.batching.max_items_per_request)
            .context("code embedding max_items_per_request too large")?,
        i64::try_from(code.batching.max_item_chars)
            .context("code embedding max_item_chars too large")?,
        i64::try_from(code.batching.max_request_chars)
            .context("code embedding max_request_chars too large")?,
        i64::try_from(code.batching.max_request_bytes)
            .context("code embedding max_request_bytes too large")?
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

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

        let embedding_batching = stored_embedding_batching(&embedding)?;
        let input_tokenizer = context69_contracts::settings::EmbeddingTokenizer::from_name(
            &embedding.input_tokenizer,
        )
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the persisted embedding input tokenizer {:?} is not supported",
                embedding.input_tokenizer
            )
        })?;
        let input_policy = context69_contracts::settings::EmbeddingInputPolicy {
            tokenizer: input_tokenizer,
            tokenizer_sha256: embedding.input_tokenizer_sha256,
            max_input_tokens: embedding
                .max_input_tokens
                .map(|max| {
                    usize::try_from(max).context("embedding max_input_tokens must be non-negative")
                })
                .transpose()?,
            document_server_prefix: embedding.document_server_prefix,
            query_server_prefix: embedding.query_server_prefix,
            add_special_tokens: embedding.add_special_tokens,
        };
        input_policy.validate().map_err(|reason| {
            anyhow::anyhow!("the persisted embedding input policy is invalid: {reason}")
        })?;
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
                // A blank query model is the document model, so it is read as
                // no query model at all.
                query_model: embedding
                    .query_model
                    .map(|model| model.trim().to_string())
                    .filter(|model| !model.is_empty()),
                dimensions: usize::try_from(embedding.dimensions)
                    .context("runtime embedding dimensions must be non-negative")?,
                timeout_secs: u64::try_from(embedding.timeout_secs)
                    .context("runtime embedding timeout_secs must be non-negative")?,
                batching: embedding_batching,
                input_policy,
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
                unit: stored_chunking_unit(&chunking.unit)?,
                tokenizer: stored_chunking_tokenizer(
                    &chunking.tokenizer,
                    chunking.tokenizer_sha256,
                )?,
                max_tokens: usize::try_from(chunking.max_tokens)
                    .context("runtime chunking max_tokens must be non-negative")?,
                overlap_tokens: usize::try_from(chunking.overlap_tokens)
                    .context("runtime chunking overlap_tokens must be non-negative")?,
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

    /// The stored optional code embedding settings, or `None` when the
    /// deployment never wrote a code row.
    pub async fn get_runtime_code_embedding_settings(
        &self,
    ) -> Result<Option<StoredRuntimeCodeEmbeddingSettings>> {
        let Some(row) = sqlx::query_file_as!(
            RuntimeCodeEmbeddingSettingsRow,
            "src/sql/db/runtime_settings/get_runtime_code_embedding_settings.sql"
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let defaults = EmbeddingBatchingSettings::default();
        Ok(Some(StoredRuntimeCodeEmbeddingSettings {
            enabled: row.enabled,
            base_url: row.base_url.unwrap_or_default(),
            model: row.model.unwrap_or_default(),
            query_model: row
                .query_model
                .map(|model| model.trim().to_string())
                .filter(|model| !model.is_empty()),
            dimensions: optional_positive_usize(row.dimensions, "code embedding dimensions")?,
            timeout_secs: optional_positive_u64(row.timeout_secs, "code embedding timeout_secs")?,
            api_key: None,
            batching: EmbeddingBatchingSettings {
                max_items_per_request: stored_batching_limit(
                    row.max_items_per_request,
                    defaults.max_items_per_request,
                )?,
                max_item_chars: stored_batching_limit(row.max_item_chars, defaults.max_item_chars)?,
                max_request_chars: stored_batching_limit(
                    row.max_request_chars,
                    defaults.max_request_chars,
                )?,
                max_request_bytes: stored_batching_limit(
                    row.max_request_bytes,
                    defaults.max_request_bytes,
                )?,
            },
        }))
    }

    /// Saves only the code embedding row.
    ///
    /// The settings service uses the combined save so document and code updates
    /// commit together; this entry point exists for the independent read/save
    /// shape and for focused round-trip verification.
    pub async fn save_runtime_code_embedding_settings(
        &self,
        code: &StoredRuntimeCodeEmbeddingSettings,
    ) -> Result<StoredRuntimeCodeEmbeddingSettings> {
        let mut tx = self.pool.begin().await?;
        write_runtime_code_embedding_settings_in(&mut tx, code).await?;
        tx.commit().await?;
        self.get_runtime_code_embedding_settings()
            .await?
            .context("runtime code embedding settings missing after save")
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
        self.save_runtime_settings_with_code(settings, None, acknowledgment)
            .await
    }

    /// Saves the runtime settings family and, when present, the independent code
    /// embedding row in the same transaction.
    ///
    /// A `None` code row skips the code SQL entirely, so an update that omitted
    /// the block preserves the stored row and credential. A present row commits
    /// atomically with the document settings: neither side can be stored while
    /// the other fails.
    pub async fn save_runtime_settings_with_code(
        &self,
        settings: &StoredRuntimeSettings,
        code: Option<&StoredRuntimeCodeEmbeddingSettings>,
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
        let embedding_dimensions = i64::try_from(settings.embedding.dimensions)
            .context("embedding dimensions too large")?;
        let embedding_timeout_secs = i64::try_from(settings.embedding.timeout_secs)
            .context("embedding timeout too large")?;
        let embedding_max_items_per_request =
            i64::try_from(settings.embedding.batching.max_items_per_request)
                .context("embedding max_items_per_request too large")?;
        let embedding_max_item_chars = i64::try_from(settings.embedding.batching.max_item_chars)
            .context("embedding max_item_chars too large")?;
        let embedding_max_request_chars =
            i64::try_from(settings.embedding.batching.max_request_chars)
                .context("embedding max_request_chars too large")?;
        let embedding_max_request_bytes =
            i64::try_from(settings.embedding.batching.max_request_bytes)
                .context("embedding max_request_bytes too large")?;
        let embedding_max_input_tokens = settings
            .embedding
            .input_policy
            .max_input_tokens
            .map(|max| i64::try_from(max).context("embedding max_input_tokens too large"))
            .transpose()?;
        let scheduler_interval_secs = i64::try_from(settings.scheduler.interval_secs)
            .context("scheduler interval too large")?;
        let scheduler_max_concurrency = i64::try_from(settings.scheduler.max_concurrency)
            .context("scheduler max_concurrency too large")?;
        let chunking_max_chars =
            i64::try_from(settings.chunking.max_chars).context("chunking max_chars too large")?;
        let chunking_overlap_chars = i64::try_from(settings.chunking.overlap_chars)
            .context("chunking overlap_chars too large")?;
        let chunking_max_tokens =
            i64::try_from(settings.chunking.max_tokens).context("chunking max_tokens too large")?;
        let chunking_overlap_tokens = i64::try_from(settings.chunking.overlap_tokens)
            .context("chunking overlap_tokens too large")?;
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
            settings.embedding.query_model,
            embedding_dimensions,
            embedding_timeout_secs,
            embedding_max_items_per_request,
            embedding_max_item_chars,
            embedding_max_request_chars,
            embedding_max_request_bytes,
            embedding_max_input_tokens,
            settings.embedding.input_policy.document_server_prefix,
            settings.embedding.input_policy.query_server_prefix,
            settings.embedding.input_policy.add_special_tokens,
            settings.embedding.input_policy.tokenizer.name(),
            settings.embedding.input_policy.tokenizer_sha256
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
            chunking_overlap_chars,
            chunking_unit_value(&settings.chunking.unit),
            settings.chunking.tokenizer.name(),
            settings.chunking.tokenizer.sha256(),
            chunking_max_tokens,
            chunking_overlap_tokens
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

        if let Some(code) = code {
            write_runtime_code_embedding_settings_in(&mut tx, code).await?;
        }

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

#[cfg(test)]
mod code_embedding_tests {
    use super::*;

    fn code_settings(enabled: bool) -> StoredRuntimeCodeEmbeddingSettings {
        StoredRuntimeCodeEmbeddingSettings {
            enabled,
            base_url: "https://embedding.invalid/v1".to_string(),
            model: "code-model".to_string(),
            query_model: Some("code-query".to_string()),
            dimensions: 1024,
            timeout_secs: 30,
            api_key: Some("in-memory-only".to_string()),
            batching: EmbeddingBatchingSettings::default(),
        }
    }

    /// The table and both read and save paths against a migrated database.
    ///
    /// The singleton row is removed before and after, so the test leaves the
    /// reusable issue database as it found it.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn code_embedding_settings_round_trip_and_disabled_retention() {
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .expect("connect test database");
        sqlx::query("DELETE FROM context69.runtime_code_embedding_settings")
            .execute(db.pool())
            .await
            .expect("clear the singleton row");

        let saved = db
            .save_runtime_code_embedding_settings(&code_settings(true))
            .await
            .expect("save the enabled row");
        assert!(saved.enabled);
        assert_eq!(saved.base_url, "https://embedding.invalid/v1");
        assert_eq!(saved.query_model.as_deref(), Some("code-query"));
        assert_eq!(saved.dimensions, 1024);
        assert!(saved.api_key.is_none(), "a stored row never carries a key");

        let disabled = db
            .save_runtime_code_embedding_settings(&code_settings(false))
            .await
            .expect("disable the row");
        assert!(!disabled.enabled);
        assert_eq!(
            disabled.base_url, "https://embedding.invalid/v1",
            "disabling retains the non-secret configuration"
        );

        sqlx::query("DELETE FROM context69.runtime_code_embedding_settings")
            .execute(db.pool())
            .await
            .expect("remove the test row");
    }
}
