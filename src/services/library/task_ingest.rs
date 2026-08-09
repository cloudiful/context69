use anyhow::{Context, anyhow};
use serde_json::Value;
use uuid::Uuid;

use super::dependency_runtime::{dependency_is_transient, is_configuration_error};
use super::*;

impl LibraryService {
    pub(crate) async fn prepare_file_sections_for_task(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        task_id: Uuid,
        section_payload: Option<Value>,
    ) -> Result<Value, UnifiedIngestError> {
        let file = self.task_file(file_id).await?;
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| task_failure("parsing", error, false))?;
        let uses_docling = matches!(
            kind,
            LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx
        ) && section_payload.is_none();
        let docling_permit = if uses_docling {
            Some(
                self.acquire_docling_permit()
                    .await
                    .map_err(|error| task_failure("docling", error, true))?,
            )
        } else {
            None
        };
        let sections = if let Some(payload) = section_payload {
            serde_json::from_value::<Vec<IngestSection>>(payload)
                .map_err(|error| task_failure("parsing", error, false))?
        } else {
            let bytes = self
                .read_active_storage_for_lease(&file.storage_rel_path, lease_token)
                .await
                .map_err(|error| task_failure("storage", error, true))?
                .with_context(|| format!("stored file not found for file {file_id}"))
                .map_err(|error| task_failure("storage", error, false))?;
            match kind {
                LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx => {
                    self.convert_unified_docling(
                        &file,
                        bytes,
                        task_id,
                        docling_permit.expect("Docling file conversion has a permit"),
                    )
                    .await
                }
                LibraryFileKind::PlainText => self.ingest_text(&file, &bytes).await,
            }
            .map_err(normalize_task_failure)?
        };
        if uses_docling {
            self.note_dependency_success(LibraryDependency::Docling, lease_token)
                .await;
        }
        serde_json::to_value(sections).map_err(|error| task_failure("parsing", error, false))
    }

    pub(crate) async fn persist_file_sections_for_task(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        lease_token: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        let file = self.task_file(file_id).await?;
        let sections = serde_json::from_value::<Vec<IngestSection>>(section_payload.clone())
            .map_err(|error| task_failure("parsing", error, false))?;
        let prepared = self
            .prepare_sections(&file, sections)
            .await
            .map_err(normalize_task_failure)?;
        self.persist_sections(&file, prepared)
            .await
            .map_err(normalize_task_failure)?;
        self.note_dependency_success(LibraryDependency::EmbeddingVector, lease_token)
            .await;
        self.store
            .update_file_status(file_id, LibraryIngestStatus::Succeeded, None, true)
            .await
            .map_err(|error| task_failure("finalize", error, true))?
            .context("file disappeared while finalizing task ingest")
            .map_err(|error| task_failure("finalize", error, false))?;
        Ok(())
    }

    pub(crate) async fn mark_file_running_for_task(
        &self,
        file_id: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        self.store
            .update_file_status(file_id, LibraryIngestStatus::Running, None, false)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .context("file disappeared while starting task ingest")
            .map_err(|error| task_failure("storage", error, false))?;
        Ok(())
    }

    pub(crate) async fn handle_task_ingest_failure(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        failure: UnifiedIngestError,
    ) -> UnifiedIngestError {
        if failure.retryable {
            if let Err(error) = self.cleanup_ingest_artifacts(file_id).await {
                return task_failure_with_dependency("indexing", error, "embedding_vector");
            }
            if let Some(dependency) = failure.dependency_key.as_deref()
                && let Ok(dependency) = dependency.parse::<LibraryDependency>()
            {
                self.note_dependency_failure_with_lease(
                    dependency,
                    lease_token,
                    &anyhow!(failure.message.clone()),
                )
                .await;
            }
            let _ = self
                .store
                .update_file_status(
                    file_id,
                    LibraryIngestStatus::Pending,
                    Some(&failure.message),
                    false,
                )
                .await;
        } else {
            let _ = self
                .store
                .update_file_status(
                    file_id,
                    LibraryIngestStatus::Failed,
                    Some(&failure.message),
                    false,
                )
                .await;
        }
        failure
    }

    pub(crate) async fn enqueue_file_translations_for_task(&self, file_id: Uuid) -> Result<()> {
        self.enqueue_file_translations(file_id).await
    }

    pub(crate) async fn enqueue_file_extractions_for_task(&self, file_id: Uuid) -> Result<()> {
        self.enqueue_file_extractions(file_id).await
    }

    async fn task_file(
        &self,
        file_id: Uuid,
    ) -> Result<crate::domain::LibraryFileRecord, UnifiedIngestError> {
        self.store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .with_context(|| format!("unknown file {file_id}"))
            .map_err(|error| task_failure("storage", error, false))
    }
}

pub(crate) fn normalize_task_failure(failure: IngestFailure) -> UnifiedIngestError {
    let mut failure = failure;
    if failure.dependency.is_none() {
        failure.dependency = super::unified_ingest::infer_unified_dependency(&failure);
    }
    if let Some(dependency) = failure.dependency {
        failure.retryable |= dependency_is_transient(dependency, &failure.error)
            || is_configuration_error(&failure.error);
    }
    UnifiedIngestError::from_failure(failure)
}

pub(crate) fn task_failure(
    stage: &str,
    error: impl Into<anyhow::Error>,
    retryable: bool,
) -> UnifiedIngestError {
    UnifiedIngestError {
        stage: stage.to_string(),
        dependency_key: None,
        retryable,
        message: error.into().to_string(),
    }
}

pub(crate) fn task_failure_with_dependency(
    stage: &str,
    error: anyhow::Error,
    dependency_key: &str,
) -> UnifiedIngestError {
    UnifiedIngestError {
        stage: stage.to_string(),
        dependency_key: Some(dependency_key.to_string()),
        retryable: true,
        message: error.to_string(),
    }
}

impl std::str::FromStr for LibraryDependency {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "s3" => Ok(Self::S3),
            "docling" => Ok(Self::Docling),
            "embedding_vector" => Ok(Self::EmbeddingVector),
            other => Err(anyhow!("unknown library dependency {other}")),
        }
    }
}
