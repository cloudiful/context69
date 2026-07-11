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
        CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse, LibraryFileSummary,
        LibraryFolderNode, LibraryFolderResponse, LibraryIngestJobResponse, LibraryIngestStatus,
        LibraryResourcePageQuery, LibraryResourcePageResponse, LibraryTreeResponse,
        LibraryUploadResponse, MoveFileRequest, MoveFolderRequest, UpsertLibraryTextRequest,
    },
    db::Database,
    docling::DoclingXlsxClient,
    domain::{ChunkPayload, LibraryFileDocumentRecord, LibraryFolderRecord, SourceRecord},
    embedding::EmbeddingProvider,
    library_store::{LibraryStore, NewLibraryFile, file_to_summary, job_to_response},
    normalize::{normalize_body, normalize_record, normalize_whitespace},
    qdrant_index::QdrantIndex,
    services::settings::SettingsService,
};

mod content_objects;
mod filenames;
mod files;
mod folders;
mod ingest;
mod metadata;
mod migration;
pub use migration::StorageMigrationSummary;
pub(crate) mod object_storage;
mod resources;
mod storage;
mod tree;
mod xlsx;

const FILE_LIBRARY_SOURCE_KEY: &str = "file_library";

#[derive(Clone)]
pub struct LibraryService {
    db: Database,
    store: LibraryStore,
    runtime: Option<LibraryRuntime>,
    chunking: ChunkingConfig,
    settings: SettingsService,
    storage_root: PathBuf,
    storage: Arc<object_storage::LibraryObjectStorage>,
    max_upload_size_bytes: usize,
    max_upload_request_size_bytes: usize,
    pdf_pages_per_task: u32,
    ingest_semaphore: Arc<Semaphore>,
}

#[derive(Clone)]
struct LibraryRuntime {
    embedding: Arc<dyn EmbeddingProvider>,
    index: QdrantIndex,
}

#[derive(Debug, Clone)]
pub struct UploadedLibraryFile {
    pub folder_id: Option<Uuid>,
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
    pub declared_sha256: Option<String>,
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
    external_id: Option<String>,
    published_at: Option<chrono::NaiveDate>,
    metadata_json: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct UpsertNamedTextFileRequest {
    pub folder_id: Option<Uuid>,
    pub external_id: String,
    pub filename: String,
    pub media_type: String,
    pub content: String,
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
        embedding: Option<Arc<dyn EmbeddingProvider>>,
        index: Option<QdrantIndex>,
        chunking: ChunkingConfig,
        settings: SettingsService,
        file_library_config: FileLibraryConfig,
    ) -> Result<Self> {
        let storage = Arc::new(object_storage::LibraryObjectStorage::from_config(
            &file_library_config,
        )?);

        Ok(Self {
            db: db.clone(),
            store: LibraryStore::new(db),
            runtime: embedding
                .zip(index)
                .map(|(embedding, index)| LibraryRuntime { embedding, index }),
            chunking,
            settings,
            storage_root: file_library_config.storage_root,
            storage,
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

    fn runtime(&self) -> Result<&LibraryRuntime> {
        self.runtime
            .as_ref()
            .ok_or_else(library_runtime_unavailable)
    }
}

fn library_runtime_unavailable() -> anyhow::Error {
    anyhow!(
        "library ingest runtime is not configured; save runtime and docling settings and restart the service"
    )
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

fn merge_library_metadata(user_metadata: &Value, system_metadata: Value) -> Result<Value> {
    let Some(system_object) = system_metadata.as_object() else {
        return Err(anyhow!("system library metadata must be an object"));
    };
    let mut merged = match user_metadata {
        Value::Null => serde_json::Map::new(),
        Value::Object(map) => map.clone(),
        _ => return Err(anyhow!("metadata_json must be an object")),
    };
    for (key, value) in system_object {
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::xlsx::extract_xlsx_sections;

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
