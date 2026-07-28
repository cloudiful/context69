use bytes::Bytes;
use uuid::Uuid;

use super::{LibraryFileSummary, LibraryFileUploadMetadata, LibraryIngestJobResponse};

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

pub(super) struct UploadedLibraryFileResult {
    pub file: LibraryFileSummary,
    pub job: LibraryIngestJobResponse,
    pub created_file: bool,
    pub rollback: UploadedLibraryFileRollback,
}

pub(super) struct UploadedLibraryFileRollback {
    pub previous_file: Option<crate::domain::LibraryFileRecord>,
    pub previous_storage_object_id: Option<Uuid>,
    pub previous_translation: Option<crate::contracts::TranslationDirective>,
    pub old_storage_paths: Vec<crate::library_store::documents::StoragePathRow>,
    pub new_storage_key: Option<String>,
    pub new_storage_object_id: Option<Uuid>,
    pub created_job: bool,
    pub restore_required: bool,
}

impl UploadedLibraryFileRollback {
    pub(super) fn empty() -> Self {
        Self {
            previous_file: None,
            previous_storage_object_id: None,
            previous_translation: None,
            old_storage_paths: Vec::new(),
            new_storage_key: None,
            new_storage_object_id: None,
            created_job: false,
            restore_required: false,
        }
    }
}
