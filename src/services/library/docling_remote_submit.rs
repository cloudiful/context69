use anyhow::Context;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::UnifiedIngestError;
use super::task_ingest::task_failure;
use super::{LibraryDependency, LibraryFileKind, LibraryService, storage};

/// Docling-stage submit error carrying the `docling` dependency key, so the
/// worker parks on the `dependency` queue reason and the failure is recorded
/// against the Docling gate like the inline pipeline does.
fn docling_submit_error(error: impl Into<anyhow::Error>, retryable: bool) -> UnifiedIngestError {
    UnifiedIngestError {
        stage: "docling".to_string(),
        dependency_key: Some(LibraryDependency::Docling.as_str().to_string()),
        retryable,
        message: error.into().to_string(),
    }
}

/// Submitted remote conversion with its due time and deadline.
pub(crate) struct DoclingRemoteSubmit {
    pub remote_task_id: String,
    pub next_poll_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

impl LibraryService {
    /// Submits one file as a whole-document async Docling task and returns
    /// the resumable remote id. Holds the Docling permit only for the submit
    /// POST; polling happens in the sweep under a separate bounded semaphore
    /// so task-worker capacity is released immediately after this returns.
    pub(crate) async fn submit_docling_remote_for_task(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        task_id: Uuid,
    ) -> Result<DoclingRemoteSubmit, UnifiedIngestError> {
        let file = self
            .store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!("unknown file {file_id}"))
            })
            .map_err(|error| task_failure("storage", error, false))?;
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| task_failure("parsing", error, false))?;
        if !matches!(
            kind,
            LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx
        ) {
            return Err(task_failure(
                "parsing",
                crate::domain_errors::DomainError::invalid_argument(format!(
                    "docling submit requires pdf/docx/xlsx, got {}",
                    file.filename
                )),
                false,
            ));
        }
        let bytes = self
            .read_active_storage_for_lease(&file.storage_rel_path, lease_token)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!(
                    "stored file not found for file {file_id}"
                ))
            })
            .map_err(|error| task_failure("storage", error, false))?;
        let config = self
            .settings
            .resolve_docling_config()
            .await
            .map_err(|error| docling_submit_error(error, true))?
            .context(crate::domain_errors::DomainError::internal(
                "docling is not configured",
            ))
            .map_err(|error| docling_submit_error(error, false))?;
        let poll_interval = config.connection.poll_interval;
        let task_timeout = config.connection.task_timeout;
        let converter = self
            .load_docling_converter_for_kind(&kind)
            .await
            .map_err(|error| docling_submit_error(error, true))?;
        let input = docling_convert::InputDocument::new(&file.filename, &file.media_type, bytes);
        let permit = self
            .acquire_docling_permit()
            .await
            .map_err(|error| docling_submit_error(error, true))?;
        let handle = converter
            .submit_async(input)
            .await
            .map_err(|error| docling_submit_error(error, true))?;
        drop(permit);
        let now = Utc::now();
        let _ = task_id;
        Ok(DoclingRemoteSubmit {
            remote_task_id: handle.task_id().to_string(),
            next_poll_at: now
                + chrono::Duration::from_std(poll_interval).unwrap_or(chrono::Duration::seconds(2)),
            deadline_at: now
                + chrono::Duration::from_std(task_timeout)
                    .unwrap_or(chrono::Duration::seconds(3600)),
        })
    }
}
