use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use chrono::Utc;
use context69_translation::{TranslationCoordinator, TranslationService};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    contracts::{
        CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse, LibraryFileSummary,
        LibraryFileUploadMetadata, LibraryFolderNode, LibraryFolderResponse,
        LibraryIngestJobResponse, LibraryIngestStatus, LibraryResourcePageQuery,
        LibraryResourcePageResponse, LibraryTreeResponse, LibraryUploadResponse, MoveFileRequest,
        MoveFolderRequest, UpsertLibraryTextRequest,
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
mod remote_download;
mod remote_proxy;
mod resources;
mod storage;
mod text_creation;
mod texts;
mod tree;
mod uploads;
mod url_imports;
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
    translation: TranslationService,
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
    pub metadata: Option<LibraryFileUploadMetadata>,
    pub translation: Option<crate::contracts::TranslationDirective>,
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
    published_at: Option<chrono::DateTime<chrono::Utc>>,
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
        translation: TranslationService,
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
            translation,
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

fn library_system_metadata(
    file: &crate::domain::LibraryFileRecord,
    folder_path: &str,
    section_key: &str,
    section_label: &str,
) -> Value {
    json!({
        "is_library_file": true,
        "library_file_id": file.id,
        "library_path": folder_path,
        "library_section_key": section_key,
        "library_section_label": section_label,
        "library_filename": file.filename,
        "library_media_type": file.media_type,
    })
}

fn compose_library_metadata(
    section_metadata: &Value,
    file_metadata: &Value,
    system_metadata: Value,
) -> Result<Value> {
    let Some(system_object) = system_metadata.as_object() else {
        return Err(anyhow!("system library metadata must be an object"));
    };
    let mut merged = match section_metadata {
        Value::Null => serde_json::Map::new(),
        Value::Object(map) => map.clone(),
        _ => return Err(anyhow!("metadata_json must be an object")),
    };
    let file_object = file_metadata
        .as_object()
        .ok_or_else(|| anyhow!("file metadata_json must be an object"))?;
    for (key, value) in file_object {
        merged.insert(key.clone(), value.clone());
    }
    merged.remove("record_hash");
    for (key, value) in system_object {
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{compose_library_metadata, xlsx::extract_xlsx_sections};

    #[test]
    fn file_metadata_overrides_section_and_system_fields_cannot_be_forged() {
        let merged = compose_library_metadata(
            &json!({"score": 1, "library_file_id": "section", "section_only": true}),
            &json!({"score": 10, "library_file_id": "caller", "record_hash": "fake"}),
            json!({"library_file_id": "system", "is_library_file": true}),
        )
        .expect("metadata");

        assert_eq!(merged["score"], 10);
        assert_eq!(merged["section_only"], true);
        assert_eq!(merged["library_file_id"], "system");
        assert_eq!(merged["is_library_file"], true);
        assert!(merged.get("record_hash").is_none());
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
