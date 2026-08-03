use bytes::Bytes;
use uuid::Uuid;

use super::{LibraryFileSummary, LibraryFileUploadMetadata};

#[derive(Debug, Clone)]
pub struct UploadedLibraryFile {
    pub folder_id: Option<Uuid>,
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
    pub declared_sha256: Option<String>,
    pub metadata: Option<LibraryFileUploadMetadata>,
    pub translation: Option<crate::contracts::TranslationDirective>,
}

#[derive(Debug, Clone)]
pub(crate) struct DownloadedLibraryFile {
    pub source_url: String,
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
    pub sha256: String,
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
