use std::time::Duration;

use anyhow::Result;
use tokio::time::timeout;

use super::dependency_runtime::is_s3_error;
use super::*;

#[derive(Debug, Clone)]
pub(crate) struct UnifiedIngestError {
    pub(crate) stage: String,
    pub(crate) dependency_key: Option<String>,
    pub(crate) retryable: bool,
    pub(crate) message: String,
}

impl std::fmt::Display for UnifiedIngestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for UnifiedIngestError {}

impl UnifiedIngestError {
    pub(super) fn from_failure(failure: IngestFailure) -> Self {
        Self {
            stage: failure.stage.as_str().to_string(),
            dependency_key: failure
                .dependency
                .map(|dependency| dependency.as_str().to_string()),
            retryable: failure.retryable,
            message: failure.to_string(),
        }
    }
}

impl LibraryService {
    pub(crate) fn file_ingest_stage(
        &self,
        filename: &str,
        media_type: &str,
    ) -> anyhow::Result<&'static str> {
        Ok(match storage::detect_file_kind(filename, media_type)? {
            LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx => "docling",
            LibraryFileKind::PlainText => "embedding",
        })
    }

    pub(super) async fn convert_unified_docling(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: &bytes::Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let task_timeout = self
            .docling_task_timeout()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let result = match storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?
        {
            LibraryFileKind::Pdf => timeout(task_timeout, self.ingest_pdf(file, bytes))
                .await
                .map_err(|error| {
                    IngestFailure::new(
                        LibraryIngestFailureStage::Docling,
                        anyhow!("docling conversion timed out: {error}"),
                    )
                })??,
            LibraryFileKind::Docx => timeout(task_timeout, self.ingest_docx(file, bytes))
                .await
                .map_err(|error| {
                    IngestFailure::new(
                        LibraryIngestFailureStage::Docling,
                        anyhow!("docling conversion timed out: {error}"),
                    )
                })??,
            LibraryFileKind::Xlsx => timeout(task_timeout, self.ingest_xlsx(file, bytes))
                .await
                .map_err(|error| {
                    IngestFailure::new(
                        LibraryIngestFailureStage::Docling,
                        anyhow!("docling conversion timed out: {error}"),
                    )
                })??,
            LibraryFileKind::PlainText => self.ingest_text(file, bytes).await?,
        };
        Ok(result)
    }
}

pub(super) fn infer_unified_dependency(failure: &IngestFailure) -> Option<LibraryDependency> {
    if is_s3_error(&failure.error) {
        return Some(LibraryDependency::S3);
    }
    let message = failure.error.to_string().to_ascii_lowercase();
    if failure.stage == LibraryIngestFailureStage::Docling
        || message.contains("docling")
        || message.contains("conversion")
    {
        return Some(LibraryDependency::Docling);
    }
    if matches!(
        failure.stage,
        LibraryIngestFailureStage::Embedding | LibraryIngestFailureStage::Indexing
    ) || message.contains("embedding")
        || message.contains("qdrant")
    {
        return Some(LibraryDependency::EmbeddingVector);
    }
    None
}
