use anyhow::{Result, anyhow};

use crate::{
    contracts::{
        UpdateDoclingSettingsRequest, UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
    },
    db::StoredSearchSettings,
    support::normalize::normalize_optional_string,
};

pub(super) fn runtime_settings_request(request: &UpdateRuntimeSettingsRequest) -> Result<()> {
    if request.qdrant.url.trim().is_empty() {
        return Err(anyhow!("runtime.qdrant.url must not be empty"));
    }
    if request.qdrant.collection_name.trim().is_empty() {
        return Err(anyhow!("runtime.qdrant.collection_name must not be empty"));
    }
    if request.embedding.provider_account_key.trim().is_empty() {
        return Err(anyhow!(
            "runtime.embedding.provider_account_key must not be empty"
        ));
    }
    if request.embedding.model.trim().is_empty() {
        return Err(anyhow!("runtime.embedding.model must not be empty"));
    }
    if request.embedding.dimensions == 0 {
        return Err(anyhow!(
            "runtime.embedding.dimensions must be greater than 0"
        ));
    }
    if request.embedding.timeout_secs == 0 {
        return Err(anyhow!(
            "runtime.embedding.timeout_secs must be greater than 0"
        ));
    }
    if request.scheduler.interval_secs == 0 {
        return Err(anyhow!(
            "runtime.scheduler.interval_secs must be greater than 0"
        ));
    }
    if request.scheduler.max_concurrency == 0 {
        return Err(anyhow!(
            "runtime.scheduler.max_concurrency must be greater than 0"
        ));
    }
    if request.scheduler.job_id.trim().is_empty() {
        return Err(anyhow!("runtime.scheduler.job_id must not be empty"));
    }
    if request.chunking.max_chars == 0 {
        return Err(anyhow!("runtime.chunking.max_chars must be greater than 0"));
    }
    if request.chunking.overlap_chars >= request.chunking.max_chars {
        return Err(anyhow!(
            "runtime.chunking.overlap_chars must be smaller than runtime.chunking.max_chars"
        ));
    }
    if request.file_library.storage_root.trim().is_empty() {
        return Err(anyhow!(
            "runtime.file_library.storage_root must not be empty"
        ));
    }
    if request.file_library.max_upload_size_mb == 0 {
        return Err(anyhow!(
            "runtime.file_library.max_upload_size_mb must be greater than 0"
        ));
    }
    if request.file_library.max_upload_request_size_mb == 0 {
        return Err(anyhow!(
            "runtime.file_library.max_upload_request_size_mb must be greater than 0"
        ));
    }
    if request.file_library.max_upload_request_size_mb < request.file_library.max_upload_size_mb {
        return Err(anyhow!(
            "runtime.file_library.max_upload_request_size_mb must be greater than or equal to runtime.file_library.max_upload_size_mb"
        ));
    }
    if request.file_library.ingest_concurrency == 0 {
        return Err(anyhow!(
            "runtime.file_library.ingest_concurrency must be greater than 0"
        ));
    }
    if request.file_library.pdf_pages_per_task == 0 {
        return Err(anyhow!(
            "runtime.file_library.pdf_pages_per_task must be greater than 0"
        ));
    }
    Ok(())
}

pub(super) fn docling_request(request: &UpdateDoclingSettingsRequest) -> Result<()> {
    let base_url = request.connection.base_url.trim();
    if base_url.is_empty() {
        return Err(anyhow!("docling.base_url must not be empty"));
    }
    if request.connection.timeout_secs == 0 {
        return Err(anyhow!("docling.timeout_secs must be greater than 0"));
    }
    if request.connection.poll_interval_secs == 0 {
        return Err(anyhow!("docling.poll_interval_secs must be greater than 0"));
    }
    if normalize_optional_string(request.vlm.provider_account_key.clone()).is_none() {
        return Err(anyhow!(
            "docling.vlm.provider_account_key is required for PDF/DOCX conversion"
        ));
    }
    if normalize_optional_string(request.vlm.vlm_pipeline_model.clone()).is_none() {
        return Err(anyhow!(
            "docling.vlm.vlm_pipeline_model is required for PDF/DOCX conversion"
        ));
    }
    if normalize_optional_string(request.vlm.picture_description_model.clone()).is_none() {
        return Err(anyhow!(
            "docling.vlm.picture_description_model is required for PDF/DOCX conversion"
        ));
    }
    if normalize_optional_string(request.vlm.code_formula_model.clone()).is_none() {
        return Err(anyhow!(
            "docling.vlm.code_formula_model is required for PDF/DOCX conversion"
        ));
    }

    Ok(())
}

pub(super) fn search_request(request: &UpdateSearchSettingsRequest) -> Result<()> {
    if request.rerank_base_url.trim().is_empty() {
        return Err(anyhow!("search.rerank_base_url must not be empty"));
    }
    if request.rerank_model.trim().is_empty() {
        return Err(anyhow!("search.rerank_model must not be empty"));
    }
    if request.candidate_limit == 0 {
        return Err(anyhow!("search.candidate_limit must be greater than 0"));
    }
    if request.timeout_secs == 0 {
        return Err(anyhow!("search.timeout_secs must be greater than 0"));
    }
    Ok(())
}

pub(super) fn stored_search_settings(settings: &StoredSearchSettings) -> Result<()> {
    if settings.rerank_base_url.trim().is_empty() {
        return Err(anyhow!("search.rerank_base_url must not be empty"));
    }
    if settings.rerank_model.trim().is_empty() {
        return Err(anyhow!("search.rerank_model must not be empty"));
    }
    if settings.candidate_limit == 0 {
        return Err(anyhow!("search.candidate_limit must be greater than 0"));
    }
    if settings.timeout_secs == 0 {
        return Err(anyhow!("search.timeout_secs must be greater than 0"));
    }
    Ok(())
}
