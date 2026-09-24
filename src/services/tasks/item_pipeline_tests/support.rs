//! Real service graph and stage runner for the issue 592 pipeline tests.
//!
//! Everything here is production wiring with network runtimes disabled so the
//! tests run the real stage handlers without reaching Docling/embedding/Qdrant.

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use context69_contracts::TaskKind;
use context69_extraction::{ExtractionDependencies, ExtractionReadiness, ExtractionService};
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use uuid::Uuid;

use crate::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    db::{ClaimedItem, Database, StoredTask},
    domain::GroupRecord,
    services::{
        document_store::DocumentStoreService,
        extraction::ExtractionPublisherAdapter,
        library::{LibraryService, LibraryServiceConfig},
        namespace::NamespaceService,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::{
            TaskService, TaskServiceDependencies,
            item_file_processors::{process_file, process_text},
            item_processors::{ItemStageRunner, ProcessResult},
            item_url_processor::process_url,
        },
        translation::TranslationPublisherAdapter,
    },
};

/// `claim_items` is a global dispatcher primitive over the shared scratch
/// database, so the cases must not claim each other's rows concurrently.
/// Serialise the cases on this lock, matching `tests/task_dispatcher_fast_path`
/// and `tests/task_lease_invariant`.
pub(super) static PIPELINE_CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Readiness only; the tests never run translation or extraction.
struct NotReady;

#[async_trait]
impl TranslationReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

#[async_trait]
impl ExtractionReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// Real service graph with local storage and no network runtimes; the tests
/// never reach the docling/embedding/indexing hop.
pub(super) async fn build_service(db: &Database) -> (TaskService, PathBuf) {
    let storage_root = std::env::temp_dir().join(format!("context69-592-{}", Uuid::new_v4()));
    let chunking = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(TranslationPublisherAdapter::new(
            None,
            None,
            chunking.clone(),
        )),
        concurrency: 1,
        readiness: Arc::new(NotReady),
    });
    let extraction = ExtractionService::new(ExtractionDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(ExtractionPublisherAdapter::new(db.clone(), None)),
        concurrency: 1,
        readiness: Arc::new(NotReady),
    });
    let library = LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root: storage_root.clone(),
                max_upload_size_mb: 1,
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "issue-592-test".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
        extraction,
    )
    .await
    .expect("build library service");
    let sync = SyncService::new(db.clone(), None, None, chunking, 1, translation.clone());
    let service = TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: NamespaceService::new(db.clone()),
        document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
        library: library.clone(),
        sync: sync.clone(),
        source_folders: SourceFoldersService::new(db.clone(), library, sync),
        translation,
        concurrency: 1,
    });
    (service, storage_root)
}

/// Delegates the storage/finalize stages to the real handlers; every other
/// stage is the post-storage hop and must observe the storage stage's `file_id`.
pub(super) struct RealStorageRunner<'a> {
    pub(super) service: &'a TaskService,
    pub(super) db: &'a Database,
    pub(super) kind: TaskKind,
    pub(super) group: &'a GroupRecord,
    pub(super) task: &'a StoredTask,
    pub(super) later_stage: &'static str,
}

#[async_trait]
impl ItemStageRunner for RealStorageRunner<'_> {
    async fn run(&self, item: &mut ClaimedItem, stage: &'static str) -> Result<ProcessResult> {
        match (self.kind, stage) {
            (TaskKind::TextBatch, "storage" | "finalize") => {
                process_text(self.service, Some(self.group), self.task, item, stage).await
            }
            (TaskKind::FileBatch, "storage" | "finalize") => {
                process_file(self.service, Some(self.group), self.task, item, stage).await
            }
            (TaskKind::UrlBatch, "download" | "storage" | "finalize") => {
                process_url(self.service, Some(self.group), self.task, item, stage).await
            }
            _ => {
                assert_eq!(stage, self.later_stage, "unexpected stage {stage}");
                let file_id = item
                    .file_id
                    .expect("later stage must observe the file_id storage saved");
                let stored: Option<Uuid> =
                    sqlx::query_scalar("SELECT file_id FROM context69.task_items WHERE id = $1")
                        .bind(item.id)
                        .fetch_one(self.db.pool())
                        .await?;
                assert_eq!(
                    stored,
                    Some(file_id),
                    "task_items.file_id must match the stage snapshot"
                );
                Ok(ProcessResult::Progressed { next: "finalize" })
            }
        }
    }
}
