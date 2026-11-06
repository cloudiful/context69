use anyhow::Result;

use crate::domain_errors::DomainError;

use crate::{
    contracts::{
        CanonicalUpdateSearchSettingsRequest, UpdateDoclingSettingsRequest,
        UpdateRuntimeFileLibrarySettings, UpdateRuntimeSettingsRequest,
        UpdateSearchSettingsRequest,
    },
    db::StoredSearchSettings,
};

use context69_contracts::settings::ChunkingUnit;
use context69_contracts::settings::DOCLING_OCR_PRESET_MAX_LEN;
use context69_contracts::settings::EmbeddingBatchingSettings;

pub(super) fn runtime_settings_request(request: &UpdateRuntimeSettingsRequest) -> Result<()> {
    if request.qdrant.url.trim().is_empty() {
        return Err(DomainError::invalid_argument("runtime.qdrant.url must not be empty").into());
    }
    if request.qdrant.collection_name.trim().is_empty() {
        return Err(DomainError::invalid_argument(
            "runtime.qdrant.collection_name must not be empty",
        )
        .into());
    }
    if request.embedding.base_url.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.embedding.base_url must not be empty").into(),
        );
    }
    if request.embedding.model.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.embedding.model must not be empty").into(),
        );
    }
    if request.embedding.dimensions == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.embedding.dimensions must be greater than 0",
        )
        .into());
    }
    if request.embedding.timeout_secs == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.embedding.timeout_secs must be greater than 0",
        )
        .into());
    }
    embedding_batching_request(&request.embedding.batching)?;
    if request.scheduler.interval_secs == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.scheduler.interval_secs must be greater than 0",
        )
        .into());
    }
    if request.scheduler.max_concurrency == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.scheduler.max_concurrency must be greater than 0",
        )
        .into());
    }
    if request.scheduler.job_id.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.scheduler.job_id must not be empty").into(),
        );
    }
    if request.chunking.max_chars == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.chunking.max_chars must be greater than 0",
        )
        .into());
    }
    if request.chunking.overlap_chars >= request.chunking.max_chars {
        return Err(DomainError::invalid_argument(
            "runtime.chunking.overlap_chars must be smaller than runtime.chunking.max_chars",
        )
        .into());
    }
    // The token integers are persisted as PostgreSQL BIGINT in both modes, so a
    // value the column cannot hold must be refused here, before any settings or
    // secret write, instead of failing the settings transaction after the
    // secret patches were already committed. The active-mode rules below still
    // apply on top of this storage check.
    if i64::try_from(request.chunking.max_tokens).is_err() {
        return Err(DomainError::invalid_argument(format!(
            "runtime.chunking.max_tokens must be at most {}",
            i64::MAX
        ))
        .into());
    }
    if i64::try_from(request.chunking.overlap_tokens).is_err() {
        return Err(DomainError::invalid_argument(format!(
            "runtime.chunking.overlap_tokens must be at most {}",
            i64::MAX
        ))
        .into());
    }
    if request.chunking.unit == ChunkingUnit::Tokens {
        if request.chunking.max_tokens == 0 {
            return Err(DomainError::invalid_argument(
                "runtime.chunking.max_tokens must be greater than 0",
            )
            .into());
        }
        if request.chunking.overlap_tokens >= request.chunking.max_tokens {
            return Err(DomainError::invalid_argument(
                "runtime.chunking.overlap_tokens must be smaller than runtime.chunking.max_tokens",
            )
            .into());
        }
    }
    if request.file_library.storage_root.trim().is_empty() {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.storage_root must not be empty",
        )
        .into());
    }
    if request.file_library.max_upload_size_mb == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.max_upload_size_mb must be greater than 0",
        )
        .into());
    }
    if request.file_library.max_upload_request_size_mb == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.max_upload_request_size_mb must be greater than 0",
        )
        .into());
    }
    if request.file_library.max_upload_request_size_mb < request.file_library.max_upload_size_mb {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.max_upload_request_size_mb must be greater than or equal to runtime.file_library.max_upload_size_mb",
        )
        .into());
    }
    if request.file_library.ingest_concurrency == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.ingest_concurrency must be greater than 0",
        )
        .into());
    }
    if request.file_library.url_import_concurrency == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.url_import_concurrency must be greater than 0",
        )
        .into());
    }
    if request.file_library.url_import_min_interval_ms == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.file_library.url_import_min_interval_ms must be greater than 0",
        )
        .into());
    }
    if let Some(s3) = &request.file_library.s3 {
        for (name, value) in [
            ("endpoint", s3.endpoint.as_str()),
            ("region", s3.region.as_str()),
            ("bucket", s3.bucket.as_str()),
            ("access_key", s3.access_key.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(DomainError::invalid_argument(format!(
                    "runtime.file_library.s3.{name} must not be empty"
                ))
                .into());
            }
        }
    }
    registry_capacity_request(&request.file_library)?;
    Ok(())
}

/// The batching limits a request carries.
///
/// They are persisted as PostgreSQL BIGINT and bound every provider request, so
/// a limit that is not positive or not representable is refused here, before any
/// secret or settings write.
fn embedding_batching_request(batching: &EmbeddingBatchingSettings) -> Result<()> {
    for (name, value) in [
        ("max_items_per_request", batching.max_items_per_request),
        ("max_item_chars", batching.max_item_chars),
        ("max_request_chars", batching.max_request_chars),
        ("max_request_bytes", batching.max_request_bytes),
    ] {
        if value == 0 {
            return Err(DomainError::invalid_argument(format!(
                "runtime.embedding.batching.{name} must be greater than 0"
            ))
            .into());
        }
        if i64::try_from(value).is_err() {
            return Err(DomainError::invalid_argument(format!(
                "runtime.embedding.batching.{name} must be at most {}",
                i64::MAX
            ))
            .into());
        }
    }
    Ok(())
}

/// The registry capacity caps a request carries.
///
/// A cap the request omits keeps the stored one, so only the caps it carries
/// are compared here. The save compares the effective policy against the
/// stored one, so a request that mixes a new cap with a stored cap is still
/// refused before anything is written.
fn registry_capacity_request(file_library: &UpdateRuntimeFileLibrarySettings) -> Result<()> {
    let caps = [
        (
            "registry_max_ids_total",
            file_library.registry_max_ids_total,
        ),
        (
            "registry_max_ids_per_file",
            file_library.registry_max_ids_per_file,
        ),
        (
            "registry_max_ids_per_run",
            file_library.registry_max_ids_per_run,
        ),
    ];
    for (name, value) in caps {
        if let Some(value) = value
            && value <= 0
        {
            return Err(DomainError::invalid_argument(format!(
                "runtime.file_library.{name} must be greater than 0"
            ))
            .into());
        }
    }
    let ordering = [
        (
            "registry_max_ids_per_run",
            file_library.registry_max_ids_per_run,
            "registry_max_ids_per_file",
            file_library.registry_max_ids_per_file,
        ),
        (
            "registry_max_ids_per_file",
            file_library.registry_max_ids_per_file,
            "registry_max_ids_total",
            file_library.registry_max_ids_total,
        ),
    ];
    for (smaller_name, smaller, larger_name, larger) in ordering {
        if let (Some(smaller), Some(larger)) = (smaller, larger)
            && smaller > larger
        {
            return Err(DomainError::invalid_argument(format!(
                "runtime.file_library.{smaller_name} must not exceed runtime.file_library.{larger_name}"
            ))
            .into());
        }
    }
    Ok(())
}

pub(super) fn docling_request(request: &UpdateDoclingSettingsRequest) -> Result<()> {
    let base_url = request.connection.base_url.trim();
    if base_url.is_empty() {
        return Err(DomainError::invalid_argument("docling.base_url must not be empty").into());
    }
    if request.connection.timeout_secs == 0 {
        return Err(
            DomainError::invalid_argument("docling.timeout_secs must be greater than 0").into(),
        );
    }
    if request.connection.poll_interval_secs == 0 {
        return Err(DomainError::invalid_argument(
            "docling.poll_interval_secs must be greater than 0",
        )
        .into());
    }
    if request.connection.task_timeout_secs == 0 {
        return Err(DomainError::invalid_argument(
            "docling.task_timeout_secs must be greater than 0",
        )
        .into());
    }
    if request.connection.max_inflight < context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN
        || request.connection.max_inflight > context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX
    {
        return Err(DomainError::invalid_argument(format!(
            "docling.max_inflight must be between {} and {}",
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX
        ))
        .into());
    }
    match &request.ocr_preset {
        // An old client that does not know the setting keeps the stored one.
        None => {}
        Some(None) => {
            return Err(
                DomainError::invalid_argument("docling.ocr_preset must not be null").into(),
            );
        }
        Some(Some(preset)) => {
            let preset = preset.trim();
            if preset.is_empty() {
                return Err(
                    DomainError::invalid_argument("docling.ocr_preset must not be empty").into(),
                );
            }
            if preset.len() > DOCLING_OCR_PRESET_MAX_LEN {
                return Err(DomainError::invalid_argument(format!(
                    "docling.ocr_preset must be at most {DOCLING_OCR_PRESET_MAX_LEN} characters"
                ))
                .into());
            }
            if !preset.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            }) {
                return Err(DomainError::invalid_argument(
                    "docling.ocr_preset must contain only ASCII letters, digits, underscore, hyphen, or dot",
                )
                .into());
            }
        }
    }

    Ok(())
}

pub(super) fn search_request(request: &UpdateSearchSettingsRequest) -> Result<()> {
    let canonical = CanonicalUpdateSearchSettingsRequest::from(request.clone());
    canonical_search_request(&canonical)
}

pub(super) fn canonical_search_request(
    request: &CanonicalUpdateSearchSettingsRequest,
) -> Result<()> {
    if request.rerank_base_url.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("search.rerank_base_url must not be empty").into(),
        );
    }
    if request.rerank_model.trim().is_empty() {
        return Err(DomainError::invalid_argument("search.rerank_model must not be empty").into());
    }
    if request.candidate_limit == 0 {
        return Err(
            DomainError::invalid_argument("search.candidate_limit must be greater than 0").into(),
        );
    }
    if request.timeout_secs == 0 {
        return Err(
            DomainError::invalid_argument("search.timeout_secs must be greater than 0").into(),
        );
    }
    validate_fusion_weights(request.vector_weight, request.keyword_weight)?;
    Ok(())
}

pub(super) fn stored_search_settings(settings: &StoredSearchSettings) -> Result<()> {
    if settings.rerank_base_url.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("search.rerank_base_url must not be empty").into(),
        );
    }
    if settings.rerank_model.trim().is_empty() {
        return Err(DomainError::invalid_argument("search.rerank_model must not be empty").into());
    }
    if settings.candidate_limit == 0 {
        return Err(
            DomainError::invalid_argument("search.candidate_limit must be greater than 0").into(),
        );
    }
    if settings.timeout_secs == 0 {
        return Err(
            DomainError::invalid_argument("search.timeout_secs must be greater than 0").into(),
        );
    }
    validate_fusion_weights(settings.vector_weight, settings.keyword_weight)?;
    Ok(())
}

/// Hybrid fusion weights must stay in [0, 1] and may not exceed the unit
/// budget together: the boost weight is the residual margin
/// (1 - vector - keyword), so it cannot be negative.
fn validate_fusion_weights(vector_weight: f32, keyword_weight: f32) -> Result<()> {
    if !(0.0..=1.0).contains(&vector_weight) {
        return Err(
            DomainError::invalid_argument("search.vector_weight must be between 0 and 1").into(),
        );
    }
    if !(0.0..=1.0).contains(&keyword_weight) {
        return Err(
            DomainError::invalid_argument("search.keyword_weight must be between 0 and 1").into(),
        );
    }
    if vector_weight + keyword_weight > 1.0 {
        return Err(DomainError::invalid_argument(
            "search.vector_weight and search.keyword_weight must not sum above 1",
        )
        .into());
    }
    Ok(())
}
