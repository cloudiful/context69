use serde::Deserialize;
use serde_json::Value;

use super::LibraryIngestFailureStage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskSourceRef {
    StoredFile,
    StagedInput(uuid::Uuid),
}

impl TaskSourceRef {
    pub(crate) fn from_input(input: Option<uuid::Uuid>) -> Self {
        match input {
            Some(id) => Self::StagedInput(id),
            None => Self::StoredFile,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryFileKind {
    Pdf,
    Docx,
    Xlsx,
    PlainText,
}

impl LibraryFileKind {
    /// Stage that owns this kind's conversion (issue 639): Docling files go
    /// through the durable `docling` submit/park/sweep stage; plain text has
    /// no remote conversion and prepares inline at `embedding`. Single source
    /// for `file_ingest_stage` and the embedding/indexing recovery routing so
    /// no task path can silently reintroduce an inline Docling wait.
    pub(crate) fn conversion_stage(&self) -> &'static str {
        match self {
            Self::Pdf | Self::Docx | Self::Xlsx => "docling",
            Self::PlainText => "embedding",
        }
    }

    /// Recovery routing for an item that reaches embedding/indexing without
    /// a persisted section payload (issue 639): Docling kinds advance back
    /// to the durable stage; plain text is already inline at `embedding`, so
    /// `None` means "stay and prepare inline".
    pub(crate) fn recovery_stage_for_missing_sections(&self) -> Option<&'static str> {
        match self.conversion_stage() {
            "docling" => Some("docling"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryImageKind {
    Png,
    Jpeg,
    WebP,
}

impl LibraryImageKind {
    /// The canonical media type of a supported image kind.
    pub(crate) const fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
        }
    }

    /// The supported kind a declared media type names, jpeg aliases included.
    pub(crate) fn from_media_type(media_type: &str) -> Option<Self> {
        match media_type.trim().to_ascii_lowercase().as_str() {
            "image/png" => Some(Self::Png),
            "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "image/webp" => Some(Self::WebP),
            _ => None,
        }
    }

    /// The supported kind a filename extension names, jpeg aliases included.
    pub(crate) fn from_extension(extension: &str) -> Option<Self> {
        match extension.trim().to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }
}

/// What the storage layer classified an upload as, before any ingest stage.
///
/// A document kind keeps the existing ingest path unchanged; an image kind is
/// the additive classification the image model stores and links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryMediaKind {
    Document(LibraryFileKind),
    Image(LibraryImageKind),
}

impl LibraryMediaKind {
    pub(crate) fn image_kind(self) -> Option<LibraryImageKind> {
        match self {
            Self::Image(kind) => Some(kind),
            Self::Document(_) => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct IngestSection {
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
pub enum LibraryDependency {
    S3,
    Docling,
    Embedding,
    Qdrant,
    EmbeddingVector,
}

impl LibraryDependency {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Docling => "docling",
            Self::Embedding => "embedding",
            Self::Qdrant => "qdrant",
            Self::EmbeddingVector => "embedding_vector",
        }
    }

    pub fn canonical_str(self) -> &'static str {
        match self {
            Self::EmbeddingVector => "embedding",
            Self::Embedding => "embedding",
            Self::Qdrant => "qdrant",
            Self::S3 => "s3",
            Self::Docling => "docling",
        }
    }

    pub fn canonical(self) -> Self {
        match self {
            Self::EmbeddingVector => Self::Embedding,
            other => other,
        }
    }

    /// Centralized alias mapping: `embedding_vector` is a legacy alias for `embedding`.
    /// All gate lookups, health checks, and dependency waits should use this.
    pub fn canonical_key(key: &str) -> &str {
        match key {
            "embedding_vector" => "embedding",
            other => other,
        }
    }
}

#[derive(Debug)]
pub(crate) struct IngestFailure {
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

#[cfg(test)]
mod tests {
    use super::{LibraryFileKind, LibraryImageKind, LibraryMediaKind, TaskSourceRef};

    #[test]
    fn task_source_selection_preserves_explicit_staged_input() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(TaskSourceRef::from_input(None), TaskSourceRef::StoredFile);
        assert_eq!(
            TaskSourceRef::from_input(Some(id)),
            TaskSourceRef::StagedInput(id)
        );
    }

    #[test]
    fn docling_kinds_convert_at_the_durable_stage_and_text_stays_inline() {
        assert_eq!(LibraryFileKind::Pdf.conversion_stage(), "docling");
        assert_eq!(LibraryFileKind::Docx.conversion_stage(), "docling");
        assert_eq!(LibraryFileKind::Xlsx.conversion_stage(), "docling");
        assert_eq!(LibraryFileKind::PlainText.conversion_stage(), "embedding");
    }

    #[test]
    fn image_kinds_name_their_canonical_media_type() {
        for (kind, media_type) in [
            (LibraryImageKind::Png, "image/png"),
            (LibraryImageKind::Jpeg, "image/jpeg"),
            (LibraryImageKind::WebP, "image/webp"),
        ] {
            assert_eq!(kind.media_type(), media_type);
            assert_eq!(LibraryImageKind::from_media_type(media_type), Some(kind));
        }
    }

    #[test]
    fn image_kind_parsing_accepts_jpeg_aliases_and_rejects_other_types() {
        assert_eq!(
            LibraryImageKind::from_media_type(" IMAGE/JPEG "),
            Some(LibraryImageKind::Jpeg)
        );
        assert_eq!(
            LibraryImageKind::from_media_type("image/jpg"),
            Some(LibraryImageKind::Jpeg)
        );
        assert_eq!(
            LibraryImageKind::from_extension("JPG"),
            Some(LibraryImageKind::Jpeg)
        );
        assert_eq!(LibraryImageKind::from_media_type("image/gif"), None);
        assert_eq!(LibraryImageKind::from_media_type("text/plain"), None);
        assert_eq!(LibraryImageKind::from_extension("gif"), None);
        let media = LibraryMediaKind::Image(LibraryImageKind::Png);
        assert_eq!(media.image_kind(), Some(LibraryImageKind::Png));
        assert_eq!(
            LibraryMediaKind::Document(LibraryFileKind::PlainText).image_kind(),
            None
        );
    }

    /// Issue 639 regression: the embedding/indexing recovery branch (missing
    /// section payload) must send every Docling format back to the durable
    /// `docling` submit/park/sweep stage — never inline conversion — while
    /// plain text keeps preparing inline.
    #[test]
    fn missing_sections_recovery_returns_to_the_durable_docling_stage() {
        for kind in [
            LibraryFileKind::Pdf,
            LibraryFileKind::Docx,
            LibraryFileKind::Xlsx,
        ] {
            assert_eq!(
                kind.recovery_stage_for_missing_sections(),
                Some("docling"),
                "recovery for {kind:?} must re-enter durable submit/park/sweep"
            );
        }
        assert_eq!(
            LibraryFileKind::PlainText.recovery_stage_for_missing_sections(),
            None,
            "plain text has no remote conversion and stays inline"
        );
    }
}
