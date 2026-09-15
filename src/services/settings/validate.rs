use anyhow::Result;

use crate::domain_errors::DomainError;

use crate::{
    contracts::{
        CanonicalUpdateSearchSettingsRequest, UpdateDoclingSettingsRequest,
        UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
    },
    db::StoredSearchSettings,
};

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
