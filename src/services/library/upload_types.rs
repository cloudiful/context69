use bytes::Bytes;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::LibraryFileSummary;

/// The file attributes a replacement will publish together with its content.
///
/// The projection mirrors the ordinary single-file update rules, so a changed
/// upload or text keeps the same folder, external identity and metadata it
/// would have received from the in-place update. The committed row keeps its
/// old values until the atomic switch.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementFileAttributes {
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub source_uri: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub metadata_json: serde_json::Value,
}

impl ReplacementFileAttributes {
    /// The file row as the parser and the metadata composer should see it.
    pub(crate) fn project(
        &self,
        file: &crate::domain::LibraryFileRecord,
    ) -> crate::domain::LibraryFileRecord {
        let mut projected = file.clone();
        projected.folder_id = self.folder_id;
        projected.external_id = self.external_id.clone();
        projected.filename = self.filename.clone();
        projected.media_type = self.media_type.clone();
        projected.source_uri = self.source_uri.clone();
        projected.published_at = self.published_at;
        projected.metadata_json = self.metadata_json.clone();
        projected
    }
}

/// The identity and the attributes of one atomic first insert.
///
/// The file row is created with exactly these attributes, so the identity of
/// the new file is decided before the transaction and never changes afterwards.
#[derive(Debug, Clone, PartialEq)]
pub struct AtomicFirstInsertFile {
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub delete_source_after_processing: bool,
}

#[derive(Debug, Clone)]
pub struct UploadedLibraryFile {
    pub folder_id: Option<Uuid>,
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
    pub declared_sha256: Option<String>,
    pub options: crate::contracts::IngestOptions,
    pub staged_storage_object_id: Option<Uuid>,
}

pub(crate) struct UploadedLibraryFileResult {
    pub(crate) file: LibraryFileSummary,
    pub(crate) rollback: UploadedLibraryFileRollback,
}

pub(super) struct UploadedLibraryFileRollback {
    pub old_storage_paths: Vec<crate::library_store::documents::StoragePathRow>,
}

impl UploadedLibraryFileRollback {
    pub(super) fn empty() -> Self {
        Self {
            old_storage_paths: Vec::new(),
        }
    }
}
