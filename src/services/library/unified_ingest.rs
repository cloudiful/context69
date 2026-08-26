use std::time::Instant;

use anyhow::anyhow;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;
use uuid::Uuid;

use super::dependency_runtime::is_s3_error;
use super::*;
use crate::docling::MAX_DOCLING_OUTPUT_BYTES;

#[derive(Debug, Clone)]
pub struct UnifiedIngestError {
    pub stage: String,
    pub dependency_key: Option<String>,
    pub retryable: bool,
    pub message: String,
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
        bytes: bytes::Bytes,
        task_id: Uuid,
        _docling_permit: OwnedSemaphorePermit,
    ) -> IngestResult<Vec<IngestSection>> {
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
        if kind == LibraryFileKind::PlainText {
            return self.ingest_text(file, &bytes).await;
        }

        let task_timeout = self
            .docling_task_timeout()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let file_bytes = bytes.len();
        let started = Instant::now();
        let result = match kind {
            LibraryFileKind::Pdf => match timeout(task_timeout, self.ingest_pdf(file, bytes)).await
            {
                Ok(result) => result,
                Err(error) => Err(IngestFailure::new(
                    LibraryIngestFailureStage::Docling,
                    anyhow!("docling conversion timed out: {error}"),
                )),
            },
            LibraryFileKind::Docx => {
                match timeout(task_timeout, self.ingest_docx(file, bytes)).await {
                    Ok(result) => result,
                    Err(error) => Err(IngestFailure::new(
                        LibraryIngestFailureStage::Docling,
                        anyhow!("docling conversion timed out: {error}"),
                    )),
                }
            }
            LibraryFileKind::Xlsx => {
                match timeout(task_timeout, self.ingest_xlsx(file, bytes)).await {
                    Ok(result) => result,
                    Err(error) => Err(IngestFailure::new(
                        LibraryIngestFailureStage::Docling,
                        anyhow!("docling conversion timed out: {error}"),
                    )),
                }
            }
            LibraryFileKind::PlainText => unreachable!("plain text handled before Docling permit"),
        };
        let output_bytes = result
            .as_ref()
            .map(|sections| section_output_bytes(sections))
            .unwrap_or_default();
        let result = result.and_then(limit_docling_output);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        info!(
            target: "docling",
            task_id = %task_id,
            file_name = %file.filename,
            file_bytes,
            output_bytes,
            elapsed_ms,
            inflight = 1usize.saturating_sub(self.docling_slots.available_permits()),
            "docling conversion finished"
        );
        result
    }
}

fn section_output_bytes(sections: &[IngestSection]) -> usize {
    sections.iter().fold(0, |total, section| {
        total.saturating_add(section.body_text.len())
    })
}

fn limit_docling_output(sections: Vec<IngestSection>) -> IngestResult<Vec<IngestSection>> {
    let output_bytes = section_output_bytes(&sections);
    if output_bytes > MAX_DOCLING_OUTPUT_BYTES {
        return Err(IngestFailure::new(
            LibraryIngestFailureStage::Parsing,
            anyhow!(
                "docling output exceeds maximum of {MAX_DOCLING_OUTPUT_BYTES} bytes: {output_bytes} bytes"
            ),
        ));
    }
    Ok(sections)
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

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use context69_contracts::LibraryIngestFailureStage;

    use super::{IngestFailure, LibraryDependency, infer_unified_dependency};

    fn failure(stage: LibraryIngestFailureStage, error: anyhow::Error) -> IngestFailure {
        IngestFailure::new(stage, error)
    }

    #[test]
    fn qdrant_cleanup_failure_with_transport_cause_is_routed_to_embedding_vector() {
        let inner = anyhow!("status 503 service unavailable");
        let error = inner.context("qdrant library file cleanup request failed");
        let failure = failure(LibraryIngestFailureStage::Storage, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::EmbeddingVector)
        );
    }

    #[test]
    fn qdrant_timeout_during_cleanup_is_routed_to_embedding_vector() {
        let error = anyhow!("qdrant library file cleanup request timed out after 30s");
        let failure = failure(LibraryIngestFailureStage::Storage, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::EmbeddingVector)
        );
    }

    #[test]
    fn indexing_stage_qdrant_error_is_routed_to_embedding_vector() {
        let error = anyhow!("qdrant points upsert request failed: connection refused")
            .context("qdrant points upsert request failed");
        let failure = failure(LibraryIngestFailureStage::Indexing, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::EmbeddingVector)
        );
    }

    #[test]
    fn non_qdrant_storage_failure_with_no_signal_is_unrouted() {
        let error = anyhow!("object store backend is offline");
        let failure = failure(LibraryIngestFailureStage::Storage, error);

        assert_eq!(infer_unified_dependency(&failure), None);
    }
}
