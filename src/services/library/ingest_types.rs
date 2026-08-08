use serde::Deserialize;
use serde_json::Value;

use super::LibraryIngestFailureStage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LibraryFileKind {
    Pdf,
    Docx,
    Xlsx,
    PlainText,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct IngestSection {
    pub section_key: String,
    pub section_label: String,
    pub title: String,
    pub summary: Option<String>,
    pub body_text: String,
    pub source_uri: Option<String>,
    pub external_id: Option<String>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata_json: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryDependency {
    S3,
    Docling,
    EmbeddingVector,
}

impl LibraryDependency {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Docling => "docling",
            Self::EmbeddingVector => "embedding_vector",
        }
    }
}

#[derive(Debug)]
pub(super) struct IngestFailure {
    pub stage: LibraryIngestFailureStage,
    pub error: anyhow::Error,
    pub dependency: Option<LibraryDependency>,
    pub retryable: bool,
}

impl IngestFailure {
    pub fn new(stage: LibraryIngestFailureStage, error: impl Into<anyhow::Error>) -> Self {
        Self {
            stage,
            error: error.into(),
            dependency: None,
            retryable: false,
        }
    }
}

impl std::fmt::Display for IngestFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for IngestFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

pub(super) type IngestResult<T> = std::result::Result<T, IngestFailure>;

#[derive(Debug, Deserialize)]
pub(super) struct SourceConfigPreview {
    #[serde(rename = "source_key")]
    pub _source_key: String,
    #[serde(rename = "connection")]
    pub _connection: String,
    #[serde(rename = "sync_strategy")]
    pub _sync_strategy: String,
    #[serde(rename = "connector_type")]
    pub _connector_type: String,
    #[serde(rename = "base_query")]
    pub _base_query: String,
    #[serde(rename = "batch_size")]
    pub _batch_size: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct SourceRecordJson {
    pub external_id: String,
    pub title: String,
    pub body_text: String,
    pub source_uri: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub metadata_json: Value,
}

pub(super) struct PreparedIngestSection {
    pub index: usize,
    pub section: IngestSection,
    pub normalized: crate::domain::NormalizedDocument,
}
