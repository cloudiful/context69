use anyhow::{Result, anyhow};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::*;
use crate::contracts::{
    LibraryFileUploadMetadata, LibraryUrlImportJobResponse, LibraryUrlImportStatus,
};
use crate::library_store::UrlImportJobRecord;

pub(super) async fn url_import_response(
    service: &LibraryService,
    job: UrlImportJobRecord,
) -> Result<LibraryUrlImportJobResponse> {
    let file = match job.file_id {
        Some(id) => service
            .store
            .get_file_in_project(job.group_id, id)
            .await?
            .map(|file| file_to_summary(&file)),
        None => None,
    };
    let ingest_job = match job.ingest_job_id {
        Some(id) => service
            .store
            .get_job_in_project(job.group_id, id)
            .await?
            .map(job_to_response),
        None => None,
    };
    Ok(LibraryUrlImportJobResponse {
        import_job_id: job.id,
        group_path: service.store.url_import_group_path(job.group_id).await?,
        source_url: job.source_url,
        status: parse_status(&job.status)?,
        attempt_count: job.attempt_count,
        file,
        ingest_job,
        error_code: job.error_code,
        error_message: job.error_message,
        failure_stage: job.failure_stage.as_deref().map(str::parse).transpose()?,
        created_at: job.created_at,
        started_at: job.started_at,
        finished_at: job.finished_at,
        updated_at: job.updated_at,
    })
}

pub(super) fn job_metadata(job: &UrlImportJobRecord) -> LibraryFileUploadMetadata {
    LibraryFileUploadMetadata {
        external_id: job.external_id.clone(),
        source_uri: job.source_uri.clone(),
        published_at: job.published_at,
        metadata_json: job.metadata_json.clone(),
    }
}

pub(super) fn job_translation(
    job: &UrlImportJobRecord,
) -> Option<crate::contracts::TranslationDirective> {
    job.translation_provided
        .then(|| crate::contracts::TranslationDirective {
            source_locale: job.translation_source_locale.clone(),
            target_locales: job.translation_target_locales.clone(),
        })
}

pub(super) fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn parse_status(value: &str) -> Result<LibraryUrlImportStatus> {
    match value {
        "queued" => Ok(LibraryUrlImportStatus::Queued),
        "downloading" => Ok(LibraryUrlImportStatus::Downloading),
        "ingesting" => Ok(LibraryUrlImportStatus::Ingesting),
        "succeeded" => Ok(LibraryUrlImportStatus::Succeeded),
        "failed" => Ok(LibraryUrlImportStatus::Failed),
        _ => Err(anyhow!("invalid URL import status")),
    }
}

pub(super) fn url_probe_dependencies(storage_backend: &str) -> Vec<LibraryDependency> {
    (storage_backend == "s3")
        .then_some(LibraryDependency::S3)
        .into_iter()
        .collect()
}

pub(super) fn url_import_uses_dependency(
    dependency: LibraryDependency,
    job: &UrlImportJobRecord,
    storage_backend: &str,
) -> bool {
    matches!(
        (dependency, storage_backend, job.file_id),
        (LibraryDependency::S3, "s3", None)
    )
}

#[cfg(test)]
mod tests {
    use super::{LibraryDependency, url_probe_dependencies};

    #[test]
    fn url_imports_probe_storage_only_when_it_may_write_new_content() {
        assert!(url_probe_dependencies("local").is_empty());
        assert_eq!(url_probe_dependencies("s3"), vec![LibraryDependency::S3]);
    }
}
