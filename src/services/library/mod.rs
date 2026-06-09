use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use chrono::Utc;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    contracts::{
        CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse,
        LibraryFileSummary, LibraryFolderNode, LibraryFolderResponse,
        LibraryIngestJobResponse, LibraryIngestStatus, LibraryTreeResponse,
        LibraryUploadResponse, MoveFileRequest, MoveFolderRequest,
    },
    db::Database,
    docling::{DoclingClient, DoclingInputKind, DoclingOutput, DoclingRequest},
    domain::{ChunkPayload, LibraryFileDocumentRecord, LibraryFolderRecord, SourceRecord},
    embedding::EmbeddingProvider,
    library_store::{LibraryStore, NewLibraryFile, file_to_summary, job_to_response},
    normalize::{normalize_body, normalize_record, normalize_whitespace},
    qdrant_index::QdrantIndex,
    services::settings::SettingsService,
};

mod files;
mod folders;
mod ingest;
mod metadata;
mod storage;
mod tree;
mod xlsx;

const FILE_LIBRARY_SOURCE_KEY: &str = "file_library";

#[derive(Clone)]
pub struct LibraryService {
    db: Database,
    store: LibraryStore,
    embedding: Arc<dyn EmbeddingProvider>,
    index: QdrantIndex,
    chunking: ChunkingConfig,
    settings: SettingsService,
    storage_root: PathBuf,
    max_upload_size_bytes: usize,
    max_upload_request_size_bytes: usize,
    pdf_pages_per_task: u32,
    ingest_semaphore: Arc<Semaphore>,
}

#[derive(Debug, Clone)]
pub struct UploadedLibraryFile {
    pub folder_id: Option<Uuid>,
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibraryFileKind {
    Pdf,
    Docx,
    Xlsx,
    PlainText,
}

#[derive(Debug, Clone)]
struct IngestSection {
    section_key: String,
    section_label: String,
    title: String,
    summary: Option<String>,
    body_text: String,
    source_uri: Option<String>,
}

#[derive(Debug, Clone)]
struct FolderNodeSeed {
    folder: Option<LibraryFolderRecord>,
    children: Vec<Uuid>,
    files: Vec<LibraryFileSummary>,
}

impl LibraryService {
    pub fn new(
        db: Database,
        embedding: Arc<dyn EmbeddingProvider>,
        index: QdrantIndex,
        chunking: ChunkingConfig,
        settings: SettingsService,
        file_library_config: FileLibraryConfig,
    ) -> Result<Self> {
        fs::create_dir_all(&file_library_config.storage_root).with_context(|| {
            format!(
                "failed to create storage root {}",
                file_library_config.storage_root.display()
            )
        })?;

        Ok(Self {
            db: db.clone(),
            store: LibraryStore::new(db),
            embedding,
            index,
            chunking,
            settings,
            storage_root: file_library_config.storage_root,
            max_upload_size_bytes: file_library_config.max_upload_size_mb * 1024 * 1024,
            max_upload_request_size_bytes: file_library_config.max_upload_request_size_mb
                * 1024
                * 1024,
            pdf_pages_per_task: file_library_config.pdf_pages_per_task,
            ingest_semaphore: Arc::new(Semaphore::new(file_library_config.ingest_concurrency)),
        })
    }

    pub fn max_upload_size_bytes(&self) -> usize {
        self.max_upload_size_bytes
    }

    pub fn max_upload_request_size_bytes(&self) -> usize {
        self.max_upload_request_size_bytes
    }

    fn pdf_pages_per_task(&self) -> u32 {
        self.pdf_pages_per_task
    }
}

fn build_library_metadata(
    file: &crate::domain::LibraryFileRecord,
    folder_path: &str,
    section: &IngestSection,
) -> Value {
    json!({
        "is_library_file": true,
        "library_file_id": file.id,
        "library_path": folder_path,
        "library_section_key": section.section_key,
        "library_section_label": section.section_label,
        "library_filename": file.filename,
        "library_media_type": file.media_type,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{storage::build_pdf_ranges, xlsx::extract_xlsx_sections};

    #[test]
    fn pdf_ranges_cover_single_and_remainder_pages() {
        assert_eq!(build_pdf_ranges(0, 5), Vec::<(u32, u32)>::new());
        assert_eq!(build_pdf_ranges(1, 5), vec![(1, 1)]);
        assert_eq!(build_pdf_ranges(10, 5), vec![(1, 5), (6, 10)]);
        assert_eq!(build_pdf_ranges(12, 5), vec![(1, 5), (6, 10), (11, 12)]);
    }

    #[test]
    fn xlsx_sections_are_split_by_sheet_groups() {
        let json = json!({
            "groups": [
                {
                    "name": "sheet: Budget",
                    "children": [{ "$ref": "#/tables/0" }]
                },
                {
                    "name": "sheet: Risks",
                    "children": [{ "$ref": "#/tables/1" }]
                }
            ],
            "tables": [
                {
                    "data": {
                        "grid": [
                            [{ "text": "Item" }, { "text": "Amount" }],
                            [{ "text": "Ops" }, { "text": "100" }]
                        ]
                    }
                },
                {
                    "data": {
                        "grid": [
                            [{ "text": "Risk" }, { "text": "Level" }],
                            [{ "text": "Capacity" }, { "text": "High" }]
                        ]
                    }
                }
            ]
        });

        let sections = extract_xlsx_sections("report.xlsx", &json).expect("sections");

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].section_label, "Budget");
        assert_eq!(sections[0].title, "report.xlsx / Budget");
        assert!(sections[0].body_text.contains("Item | Amount"));
        assert_eq!(sections[1].section_label, "Risks");
        assert!(sections[1].body_text.contains("Capacity | High"));
    }
}
