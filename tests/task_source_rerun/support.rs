//! Shared fixtures of the source-rerun regression targets (issue 1138 P29).
//!
//! The fixtures seed the database, the library file, and the services the
//! fresh-source cases run against. They hold no test of their own, so every
//! target that includes them keeps its own test names.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::domain::{ChunkPayload, DocumentChunk, GroupRecord, LibraryFileDocumentRecord};
use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
use context69::library_store::{LibraryStore, NewLibraryFile};
use context69::qdrant_index::QdrantIndex;
use context69::services::document_store::DocumentStoreService;
use context69::services::library::test_remote::RemoteHttpTransport;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::namespace::NamespaceService;
use context69::services::secret_store;
use context69::services::settings::SettingsService;
use context69::services::source_folders::SourceFoldersService;
use context69::services::sync::SyncService;
use context69::services::tasks::{TaskService, TaskServiceDependencies};
use context69::services::translation::TranslationPublisherAdapter;
use context69::vector_runtime::{VectorRuntime, VectorSnapshot};
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};

use crate::qdrant_points_mock::QdrantPointsMock;

/// The claim path is global, so the cases of one target serialize among
/// themselves.
pub static CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// An embedding provider that fails the first calls when it is asked to.
pub struct FakeEmbedding {
    fail_first_calls: AtomicUsize,
}

impl FakeEmbedding {
    pub fn new(fail_first_calls: usize) -> Self {
        Self {
            fail_first_calls: AtomicUsize::new(fail_first_calls),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for FakeEmbedding {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let failing =
            self.fail_first_calls
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    if remaining == 0 {
                        None
                    } else {
                        Some(remaining - 1)
                    }
                });
        if failing.is_ok() {
            return Err(anyhow::anyhow!("transient embedding outage"));
        }
        Ok(texts
            .iter()
            .map(|text| vec![text.len() as f32; 4])
            .collect())
    }
}

/// A translation service that is never ready, so no translation runs.
pub struct ReadyTranslation;

#[async_trait]
impl TranslationReadiness for ReadyTranslation {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

pub async fn connect_db() -> Database {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

pub async fn seed_user(db: &Database) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("source-rerun-{}", Uuid::new_v4()))
    .bind("Source Rerun")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

pub async fn seed_group(db: &Database) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, full_path, name, visibility, kind) \
         VALUES ($1, $2, $3, 'public', 'shared') \
         RETURNING id, group_key, full_path, name, created_at, updated_at",
    )
    .bind(format!("source-rerun-{}", Uuid::new_v4()))
    .bind(format!("test/source-rerun-{}", Uuid::new_v4()))
    .bind("Source Rerun")
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    use sqlx::Row;
    GroupRecord {
        id: row.get("id"),
        parent_group_id: None,
        group_key: row.get("group_key"),
        group_path: row.get("full_path"),
        parent_group_path: None,
        name: row.get("name"),
        visibility: context69_contracts::Visibility::Public,
        kind: context69_contracts::GroupKind::Shared,
        owner_user_id: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        current_role: None,
    }
}

/// One seeded library file and the document it already owned.
pub struct SeededFile {
    pub file_id: Uuid,
    pub storage_root: PathBuf,
    /// The document the file already owned before the rerun.
    pub existing_document_id: i64,
    /// The bytes the file owned before the rerun.
    pub old_body: Vec<u8>,
}

pub async fn seed_file(db: &Database, group: &GroupRecord, with_bytes: bool) -> SeededFile {
    let storage_root = std::env::temp_dir().join(format!("source-rerun-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let body = b"fresh source body for replacement\n".to_vec();
    let digest = sha256_hex(&body);
    let storage_rel_path = format!("objects/{}.txt", Uuid::new_v4());
    if with_bytes {
        let path = storage_root.join(&storage_rel_path);
        std::fs::create_dir_all(path.parent().expect("object parent")).expect("create object dir");
        std::fs::write(&path, &body).expect("write source bytes");
    }
    let store = LibraryStore::new(db.clone());
    let file = store
        .create_file_in_project(
            group.id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: Some(format!("external-{}", Uuid::new_v4())),
                filename: "source.txt".into(),
                media_type: "text/plain".into(),
                size_bytes: body.len() as i64,
                sha256: digest,
                storage_rel_path: storage_rel_path.clone(),
                storage_object_id: None,
                delete_source_after_processing: false,
            },
        )
        .await
        .expect("seed library file");
    let existing_document_id = seed_existing_section(db, &store, group, file.id, "document").await;
    SeededFile {
        file_id: file.id,
        storage_root,
        existing_document_id,
        old_body: body,
    }
}

/// One seeded URL source file.
///
/// The file owns a real storage object and a source URL, so a fresh download
/// has something to replace and the URL path is the one the child takes.
pub async fn seed_url_source_file(
    db: &Database,
    group: &GroupRecord,
    source_uri: &str,
) -> SeededFile {
    let storage_root = std::env::temp_dir().join(format!("source-rerun-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let body = b"# stale parent source\n".to_vec();
    let digest = sha256_hex(&body);
    let object_key = format!("objects/{}/{}", group.id, digest);
    let path = storage_root.join(&object_key);
    std::fs::create_dir_all(path.parent().expect("object parent")).expect("create object dir");
    std::fs::write(&path, &body).expect("write source bytes");

    let store = LibraryStore::new(db.clone());
    let file = store
        .create_file_in_project(
            group.id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: Some(format!("external-{}", Uuid::new_v4())),
                filename: "doc.md".into(),
                media_type: "text/markdown".into(),
                size_bytes: body.len() as i64,
                sha256: digest.clone(),
                storage_rel_path: object_key.clone(),
                storage_object_id: None,
                delete_source_after_processing: false,
            },
        )
        .await
        .expect("seed library file");
    let object_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key) \
         VALUES ($1, $2, $3, $4, 'local', $5)",
    )
    .bind(object_id)
    .bind(group.id)
    .bind(&digest)
    .bind(body.len() as i64)
    .bind(&object_key)
    .execute(db.pool())
    .await
    .expect("seed the storage object");
    sqlx::query(
        "UPDATE context69.library_files SET storage_object_id = $2, source_uri = $3 \
         WHERE id = $1",
    )
    .bind(file.id)
    .bind(object_id)
    .bind(source_uri)
    .execute(db.pool())
    .await
    .expect("attach the storage object and the source URL");

    let existing_document_id = seed_existing_section(db, &store, group, file.id, "document").await;
    SeededFile {
        file_id: file.id,
        storage_root,
        existing_document_id,
        old_body: body,
    }
}

/// One seeded PDF source file.
///
/// The file owns a real storage object, so the Docling submit reads the PDF
/// bytes from the live source path instead of a saved parse.
pub async fn seed_pdf_source_file(db: &Database, group: &GroupRecord) -> SeededFile {
    let storage_root = std::env::temp_dir().join(format!("source-rerun-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let body = b"%PDF-1.7\nthe parent pdf bytes\n%%EOF\n".to_vec();
    let digest = sha256_hex(&body);
    let object_key = format!("objects/{}/{}", group.id, digest);
    let path = storage_root.join(&object_key);
    std::fs::create_dir_all(path.parent().expect("object parent")).expect("create object dir");
    std::fs::write(&path, &body).expect("write source bytes");

    let store = LibraryStore::new(db.clone());
    let file = store
        .create_file_in_project(
            group.id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: Some(format!("external-{}", Uuid::new_v4())),
                filename: "doc.pdf".into(),
                media_type: "application/pdf".into(),
                size_bytes: body.len() as i64,
                sha256: digest.clone(),
                storage_rel_path: object_key.clone(),
                storage_object_id: None,
                delete_source_after_processing: false,
            },
        )
        .await
        .expect("seed library file");
    let object_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key) \
         VALUES ($1, $2, $3, $4, 'local', $5)",
    )
    .bind(object_id)
    .bind(group.id)
    .bind(&digest)
    .bind(body.len() as i64)
    .bind(&object_key)
    .execute(db.pool())
    .await
    .expect("seed the storage object");
    sqlx::query("UPDATE context69.library_files SET storage_object_id = $2 WHERE id = $1")
        .bind(file.id)
        .bind(object_id)
        .execute(db.pool())
        .await
        .expect("attach the storage object");

    let existing_document_id = seed_existing_section(db, &store, group, file.id, "document").await;
    SeededFile {
        file_id: file.id,
        storage_root,
        existing_document_id,
        old_body: body,
    }
}

pub async fn seed_existing_section(
    db: &Database,
    store: &LibraryStore,
    group: &GroupRecord,
    file_id: Uuid,
    section_key: &str,
) -> i64 {
    let external_id = format!("existing-{}", Uuid::new_v4());
    let record_hash = "0".repeat(64);
    let mut payload = ChunkPayload {
        chunk_id: Uuid::new_v4(),
        document_id: 0,
        group_id: group.id,
        group_key: group.group_key.clone(),
        group_path: group.group_path.clone(),
        visibility: context69_contracts::Visibility::Public,
        source_key: "library".into(),
        external_id: external_id.clone(),
        title: "existing".into(),
        summary: None,
        source_uri: "context69://test".into(),
        published_at: None,
        updated_at_source: chrono::Utc::now(),
        record_hash: record_hash.clone(),
        chunk_index: 0,
        chunk_text: "existing body".into(),
        metadata_json: serde_json::json!({}),
        content_locale: "original".into(),
        source_locale: None,
        translation_provider: None,
    };
    let upserted = db.upsert_document(&payload).await.expect("seed document");
    payload.document_id = upserted.document_id;
    let chunk = DocumentChunk {
        id: Uuid::new_v4(),
        document_id: upserted.document_id,
        chunk_index: 0,
        text: "existing body".into(),
        record_hash: record_hash.clone(),
    };
    db.insert_document_chunks(upserted.document_id, &record_hash, &[chunk])
        .await
        .expect("seed chunks");
    let link = LibraryFileDocumentRecord {
        file_id,
        document_id: upserted.document_id,
        group_id: group.id,
        visibility: context69_contracts::Visibility::Public,
        section_key: section_key.into(),
        section_label: "existing".into(),
        section_external_id: None,
        section_source_uri: None,
        section_published_at: None,
        section_metadata_json: serde_json::json!({}),
        sort_order: 0,
    };
    let mut connection = db.pool().acquire().await.expect("connection");
    store
        .merge_file_documents_on(&mut connection, file_id, &[link])
        .await
        .expect("seed file link");
    upserted.document_id
}

/// One published result of a document, so the history of the parent's
/// document can be checked after the rerun.
pub async fn seed_document_result(db: &Database, document_id: i64, author_user_id: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO context69.document_results \
         (document_id, author_user_id, producer, result_kind, schema_ref, \
          source_record_hash, idempotency_key, result) \
         VALUES ($1, $2, 'test-producer', 'summary', 'test://result', $3, $4, $5) \
         RETURNING id",
    )
    .bind(document_id)
    .bind(author_user_id)
    .bind("0".repeat(64))
    .bind(format!("result-{}", Uuid::new_v4()))
    .bind(serde_json::json!({ "summary": "the parent result" }))
    .fetch_one(db.pool())
    .await
    .expect("seed the document result")
}

pub async fn seed_succeeded_source(
    db: &Database,
    user: i64,
    group: &GroupRecord,
    file: &SeededFile,
    payload: serde_json::Value,
    kind: &str,
) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (_, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id: user,
            group_id: Some(group.id),
            kind,
            group_path: Some("test/source-rerun"),
            source_key: None,
            payloads: &[payload],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("source-rerun-{}", Uuid::new_v4()),
        })
        .await
        .expect("create source task");
    sqlx::query(
        "UPDATE context69.task_items SET file_id = $2, status = 'succeeded', \
         finished_at = now(), lease_token = NULL, lease_until = NULL WHERE id = $1",
    )
    .bind(item_ids[0])
    .bind(file.file_id)
    .execute(db.pool())
    .await
    .expect("mark source item succeeded");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'succeeded', succeeded_count = 1, \
         queued_count = 0, finished_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("mark source task succeeded");
    (task_id, item_ids[0])
}

pub fn source_payload(file: &SeededFile) -> serde_json::Value {
    let _ = file;
    serde_json::json!({
        "filename": "source.txt",
        "media_type": "text/plain",
        "options": { "metadata": {}, "source_policy": "retain" },
        "section_payload": [{ "section_key": "document", "body_text": "stale saved parse" }],
        "pipeline_progress": { "stage": "indexing", "attempt": 1 },
        "indexing_checkpoint": { "next_batch_index": 4 },
        "download_artifact": { "source_url": "https://stale.example/doc" }
    })
}

/// The payload of a URL source parent.
pub fn url_source_payload(file: &SeededFile) -> serde_json::Value {
    let _ = file;
    serde_json::json!({
        "url": "https://93.184.216.34/doc.md",
        "filename": "doc.md",
        "media_type": "text/markdown",
        "options": { "metadata": {}, "source_policy": "retain" },
        "section_payload": [{ "section_key": "document", "body_text": "stale saved parse" }],
        "pipeline_progress": { "stage": "indexing", "attempt": 1 },
        "download_artifact": { "source_url": "https://stale.example/doc" }
    })
}

pub async fn build_service(
    db: &Database,
    storage_root: &Path,
    embedding: Arc<FakeEmbedding>,
    mock: &QdrantPointsMock,
    caps: Option<[i64; 3]>,
) -> TaskService {
    build_service_with_transport(db, storage_root, embedding, mock, caps, None).await
}

/// Build the task service, optionally with an injected remote transport.
///
/// The transport replaces the HTTP call of a remote download only; every guard
/// around it stays in production code.
pub async fn build_service_with_transport(
    db: &Database,
    storage_root: &Path,
    embedding: Arc<FakeEmbedding>,
    mock: &QdrantPointsMock,
    caps: Option<[i64; 3]>,
    transport: Option<Arc<dyn RemoteHttpTransport>>,
) -> TaskService {
    let chunking = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };
    let runtime = VectorRuntime::from_legacy(
        EmbeddingRuntime::new(Some(embedding.clone()), None),
        None,
        "",
    );
    let generation = runtime.generation();
    let index =
        QdrantIndex::for_test_unreachable(mock.url(), "source-rerun", 4).expect("build test index");
    assert!(runtime.publish(
        generation,
        VectorSnapshot {
            embedding: embedding.clone(),
            identity: None,
            index: Arc::new(index),
            collection: "source-rerun".into(),
            generation,
            configured_fingerprint: Some("task_source_rerun-fingerprint".to_string()),
        }
    ));
    runtime.set_configured_fingerprint(Some("source-rerun-fingerprint".to_string()));
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(TranslationPublisherAdapter::new(
            None,
            None,
            chunking.clone(),
        )),
        concurrency: 1,
        readiness: Arc::new(ReadyTranslation),
    });
    let mut library = LibraryService::with_vector_runtime(
        db.clone(),
        runtime,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root: storage_root.to_path_buf(),
                max_upload_size_mb: 10,
                max_upload_request_size_mb: 10,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: true,
            embedding_vector_configuration_fingerprint: "source-rerun".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
    )
    .await
    .expect("build library service");
    if let Some(transport) = transport {
        library = library.with_remote_transport_for_test(transport);
    }
    let sync = SyncService::new(
        db.clone(),
        secret_store::build_unkeyed(db),
        None,
        None,
        chunking,
        1,
        translation.clone(),
    );
    let service = TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: NamespaceService::new(db.clone()),
        document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
        library: library.clone(),
        sync: sync.clone(),
        source_folders: SourceFoldersService::new(db.clone(), library, sync),
        translation,
    });
    match caps {
        Some(caps) => service.with_replacement_caps_for_test(caps),
        None => service,
    }
}

pub async fn claim_item(db: &Database, task_id: Uuid) -> context69::db::ClaimedItem {
    for _ in 0..20 {
        if let Some(item) = db
            .claim_items_fast()
            .await
            .expect("claim items")
            .into_iter()
            .find(|item| item.task_id == task_id)
        {
            return item;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the forked item was never claimable");
}

pub async fn item_state(db: &Database, item_id: Uuid) -> (String, Option<String>, Option<Uuid>) {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT status, error_message, input_storage_object_id \
         FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("read item");
    (
        row.get("status"),
        row.get("error_message"),
        row.get("input_storage_object_id"),
    )
}

/// The canonical digest of some bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
