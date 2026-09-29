use super::dependency_runtime::is_s3_error;
use super::*;

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
        Ok(storage::detect_file_kind(filename, media_type)?.conversion_stage())
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
    // Qdrant is checked first: its error messages always contain "qdrant",
    // and after phase 1 they must not be collapsed into embedding.
    if message.contains("qdrant") {
        return Some(LibraryDependency::Qdrant);
    }
    if matches!(
        failure.stage,
        LibraryIngestFailureStage::Embedding | LibraryIngestFailureStage::Indexing
    ) || message.contains("embedding")
    {
        return Some(LibraryDependency::Embedding);
    }
    // Stage Storage can also be qdrant cleanup; already handled via qdrant
    // substring above. If storage error without qdrant/embedding signal, leave
    // unrouted so caller can decide not to trip a gate.
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
    fn qdrant_cleanup_failure_with_transport_cause_is_routed_to_qdrant() {
        let inner = anyhow!("status 503 service unavailable");
        let error = inner.context("qdrant library file cleanup request failed");
        let failure = failure(LibraryIngestFailureStage::Storage, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::Qdrant)
        );
    }

    #[test]
    fn qdrant_timeout_during_cleanup_is_routed_to_qdrant() {
        let error = anyhow!("qdrant library file cleanup request timed out after 30s");
        let failure = failure(LibraryIngestFailureStage::Storage, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::Qdrant)
        );
    }

    #[test]
    fn indexing_stage_qdrant_error_is_routed_to_qdrant() {
        let error = anyhow!("qdrant points upsert request failed: connection refused")
            .context("qdrant points upsert request failed");
        let failure = failure(LibraryIngestFailureStage::Indexing, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::Qdrant)
        );
    }

    #[test]
    fn embedding_stage_error_is_routed_to_embedding() {
        let error = anyhow!("embedding request failed: status=503");
        let failure = failure(LibraryIngestFailureStage::Embedding, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::Embedding)
        );
    }

    #[test]
    fn embedding_message_is_routed_to_embedding_not_qdrant() {
        let error =
            anyhow!("embedding upstream transport error: operation=send request kind=connect");
        let failure = failure(LibraryIngestFailureStage::Other, error);

        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::Embedding)
        );
    }

    #[test]
    fn non_qdrant_storage_failure_with_no_signal_is_unrouted() {
        let error = anyhow!("object store backend is offline");
        let failure = failure(LibraryIngestFailureStage::Storage, error);

        assert_eq!(infer_unified_dependency(&failure), None);
    }

    #[test]
    fn qdrant_takes_precedence_over_embedding_substring() {
        let error = anyhow!("qdrant points upsert request failed: embedding dimension mismatch?")
            .context("qdrant points upsert request failed");
        let failure = failure(LibraryIngestFailureStage::Indexing, error);
        assert_eq!(
            infer_unified_dependency(&failure),
            Some(LibraryDependency::Qdrant)
        );
    }
}
