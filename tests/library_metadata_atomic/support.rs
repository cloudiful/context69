//! Fixture of the atomic metadata publication tests (issue 1138, P19).
//!
//! One fixture owns a test group, a library file with a linked document and the
//! service under test. Each case starts from that state and changes exactly one
//! thing, so a refused publication can be compared with the state it refused to
//! change.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::Database;
use context69::domain::{ChunkPayload, DocumentChunk, LibraryFileDocumentRecord, SourceRecord};
use context69::library_store::{LibraryStore, NewLibraryFile};
use context69::normalize::normalize_record;
use context69::qdrant_index::QdrantIndex;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::metadata_payload_sync::MetadataPayloadSyncWorker;
use context69::services::settings::SettingsService;
use context69::vector_runtime::{VectorRuntime, VectorSnapshot};
use context69_contracts::Visibility;
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use sqlx::Connection;
use sqlx::Row;
use uuid::Uuid;

pub const CANONICAL_BODY: &str = "alpha beta gamma delta epsilon";

struct NoopCallbacks;

/// Whether a fresh, non-pooled session can take this file's mirror lock.
pub async fn can_take_session_lock(file_id: Uuid) -> bool {
    let mut connection =
        sqlx::PgConnection::connect(&context69_test_support::accessor::database_url())
            .await
            .expect("open a fresh session");
    let locked: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_lock(hashtextextended(\
         'context69.library_file_metadata_mirror:' || $1::uuid::text, 0))",
    )
    .bind(file_id)
    .fetch_one(&mut connection)
    .await
    .expect("try the mirror lock");
    if locked {
        sqlx::query_scalar::<_, bool>(
            "SELECT pg_advisory_unlock(hashtextextended(\
             'context69.library_file_metadata_mirror:' || $1::uuid::text, 0))",
        )
        .bind(file_id)
        .fetch_one(&mut connection)
        .await
        .expect("release the mirror lock");
    }
    drop(connection);
    locked
}

/// Wait until a fresh, non-pooled session can take this file's mirror lock.
///
/// Ending a detached PostgreSQL session is asynchronous, so this is a bounded
/// observation of the release, not a timing proof of the window.
pub async fn wait_for_session_lock_release(file_id: Uuid) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !can_take_session_lock(file_id).await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the cancelled mirror session still holds the lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[async_trait]
impl TranslationPublisher for NoopCallbacks {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        _translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// The seeded group fields a document payload needs.
#[derive(Debug, Clone)]
pub struct GroupSeed {
    pub id: i64,
    pub key: String,
    pub path: String,
}

/// A seeded document with its canonical body and its overlapping chunks.
pub struct SeededDocument {
    pub document_id: i64,
    pub record_hash: String,
    pub chunk_ids: Vec<Uuid>,
    pub chunk_texts: Vec<String>,
}

pub struct MetadataFixture {
    pub db: Database,
    pub store: LibraryStore,
    pub service: Arc<LibraryService>,
    pub group: GroupSeed,
    pub file_id: Uuid,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl MetadataFixture {
    pub async fn new() -> MetadataFixture {
        MetadataFixture::new_with_caps(None).await
    }

    /// The fixture with explicit search-registry admission caps.
    ///
    /// Without the caps the metadata publisher keeps its disabled behavior: it
    /// changes the file and its documents and retires no translation point.
    pub async fn new_with_caps(caps: Option<[i64; 3]>) -> MetadataFixture {
        let guard = SUITE_LOCK.lock().await;
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap library schema");
        sqlx::query(
            "UPDATE context69.search_registry_epoch \
             SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
                 rebuild_fingerprint = NULL, rebuild_collection = NULL, \
                 rebuild_started_at = NULL, rebuild_error = NULL \
             WHERE id = 1",
        )
        .execute(db.pool())
        .await
        .expect("reset the fixture barrier");
        sqlx::query("UPDATE context69.vector_index_state SET active = FALSE WHERE active")
            .execute(db.pool())
            .await
            .expect("deactivate a stale fixture vector index");
        sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
            .execute(db.pool())
            .await
            .expect("clear stale desired qdrant settings");
        sqlx::query("DELETE FROM context69.runtime_embedding_settings")
            .execute(db.pool())
            .await
            .expect("clear stale desired embedding settings");
        let group = seed_group(&db).await;
        let store = LibraryStore::new(db.clone());
        let storage_root = std::env::temp_dir().join(format!("context69-test-{}", Uuid::new_v4()));
        let settings = SettingsService::new(db.clone());
        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(NoopCallbacks),
            concurrency: 1,
            readiness: Arc::new(NoopCallbacks),
        });
        let service = LibraryService::new(
            db.clone(),
            None,
            None,
            LibraryServiceConfig {
                chunking: ChunkingConfig {
                    max_chars: 1000,
                    overlap_chars: 100,
                },
                file_library: FileLibraryConfig {
                    storage_root: storage_root.clone(),
                    max_upload_size_mb: 1,
                    trusted_proxy_enabled: false,
                    s3: None,
                },
                valkey_url: None,
                embedding_vector_configured: false,
                embedding_vector_configuration_fingerprint: "test".to_string(),
            },
            settings,
            translation,
        )
        .await
        .expect("build library service");
        let service = match caps {
            Some(caps) => service.with_registry_caps_for_test(caps),
            None => service,
        };
        let file = store
            .create_file_in_project(
                group.id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: None,
                    filename: "metadata.txt".into(),
                    media_type: "text/plain".into(),
                    size_bytes: 5,
                    sha256: "0".repeat(64),
                    storage_rel_path: format!("objects/{}", Uuid::new_v4()),
                    storage_object_id: None,
                    delete_source_after_processing: false,
                },
            )
            .await
            .expect("create fixture file");
        MetadataFixture {
            db,
            store,
            service: Arc::new(service),
            group,
            file_id: file.id,
            _guard: guard,
        }
    }

    pub fn source_record(&self, external_id: &str, body: &str) -> SourceRecord {
        let mut metadata = serde_json::Map::new();
        metadata.insert("seed".into(), serde_json::json!("baseline"));
        SourceRecord {
            external_id: external_id.into(),
            title: "Metadata Document".into(),
            summary: None,
            body_text: body.into(),
            source_uri: "context69://test".into(),
            published_at: None,
            updated_at: chrono::Utc::now(),
            metadata_json: serde_json::Value::Object(metadata),
        }
    }

    /// A document whose current version body is `body` and whose stored chunks
    /// overlap, exactly like a real multi-chunk ingest.
    pub async fn seed_document(
        &self,
        section_key: &str,
        sort_order: i32,
        body: &str,
        chunks: &[&str],
    ) -> SeededDocument {
        let external_id = format!("{section_key}-{}", Uuid::new_v4());
        let source = self.source_record(&external_id, body);
        let record_hash = normalize_record(source.clone()).record_hash;
        let payload = ChunkPayload {
            chunk_id: Uuid::nil(),
            document_id: 0,
            group_id: self.group.id,
            group_key: self.group.key.clone(),
            group_path: self.group.path.clone(),
            visibility: Visibility::Private,
            source_key: "library".into(),
            external_id,
            title: source.title.clone(),
            summary: source.summary.clone(),
            source_uri: source.source_uri.clone(),
            published_at: source.published_at,
            updated_at_source: source.updated_at,
            record_hash: record_hash.clone(),
            chunk_index: 0,
            chunk_text: body.to_string(),
            metadata_json: source.metadata_json.clone(),
            content_locale: "original".into(),
            source_locale: None,
            translation_provider: None,
        };
        let upserted = self
            .db
            .upsert_document(&payload)
            .await
            .expect("seed document");
        let mut chunk_ids = Vec::new();
        let mut chunk_texts = Vec::new();
        let mut rows = Vec::new();
        for (index, text) in chunks.iter().enumerate() {
            let chunk_id = Uuid::new_v4();
            chunk_ids.push(chunk_id);
            chunk_texts.push((*text).to_string());
            rows.push(DocumentChunk {
                id: chunk_id,
                document_id: upserted.document_id,
                chunk_index: i32::try_from(index).expect("chunk index"),
                text: (*text).to_string(),
                record_hash: record_hash.clone(),
            });
        }
        self.db
            .insert_document_chunks(upserted.document_id, &record_hash, &rows)
            .await
            .expect("seed document chunks");
        let mut connection = self.db.pool().acquire().await.expect("acquire");
        self.store
            .merge_file_documents_on(
                &mut connection,
                self.file_id,
                &[LibraryFileDocumentRecord {
                    file_id: self.file_id,
                    document_id: upserted.document_id,
                    group_id: self.group.id,
                    visibility: Visibility::Private,
                    section_key: section_key.into(),
                    section_label: section_key.into(),
                    section_external_id: None,
                    section_source_uri: None,
                    section_published_at: None,
                    section_metadata_json: serde_json::json!({}),
                    sort_order,
                }],
            )
            .await
            .expect("link file document");
        SeededDocument {
            document_id: upserted.document_id,
            record_hash,
            chunk_ids,
            chunk_texts,
        }
    }

    pub async fn create_folder(&self, name: &str) -> Uuid {
        let folder_id = Uuid::new_v4();
        self.store
            .create_folder_in_project(self.group.id, folder_id, None, name)
            .await
            .expect("create folder");
        folder_id
    }

    pub async fn create_child_folder(&self, parent_id: Uuid, name: &str) -> Uuid {
        let folder_id = Uuid::new_v4();
        self.store
            .create_folder_in_project(self.group.id, folder_id, Some(parent_id), name)
            .await
            .expect("create child folder");
        folder_id
    }

    /// Persist a desired vector configuration without publishing a runtime.
    pub async fn seed_desired_vector_settings(
        &self,
        qdrant_url: &str,
        collection: &str,
        embedding_base_url: &str,
        model: &str,
        dimensions: i64,
    ) {
        sqlx::query(
            "INSERT INTO context69.runtime_qdrant_settings \
             (singleton, url, collection_name) VALUES (TRUE, $1, $2) \
             ON CONFLICT (singleton) DO UPDATE \
             SET url = EXCLUDED.url, collection_name = EXCLUDED.collection_name",
        )
        .bind(qdrant_url)
        .bind(collection)
        .execute(self.db.pool())
        .await
        .expect("seed desired qdrant settings");
        sqlx::query(
            "INSERT INTO context69.runtime_embedding_settings \
             (singleton, base_url, model, dimensions, timeout_secs) \
             VALUES (TRUE, $1, $2, $3, 30) \
             ON CONFLICT (singleton) DO UPDATE \
             SET base_url = EXCLUDED.base_url, model = EXCLUDED.model, \
                 dimensions = EXCLUDED.dimensions",
        )
        .bind(embedding_base_url)
        .bind(model)
        .bind(dimensions)
        .execute(self.db.pool())
        .await
        .expect("seed desired embedding settings");
    }

    pub async fn seed_active_vector_state(
        &self,
        collection: &str,
        fingerprint: &str,
        dimensions: i64,
    ) {
        sqlx::query("UPDATE context69.vector_index_state SET active = FALSE WHERE active")
            .execute(self.db.pool())
            .await
            .expect("clear the previous active pointer");
        sqlx::query(
            "INSERT INTO context69.vector_index_state \
             (collection_name, fingerprint, embedding_base_url, embedding_model, \
              dimensions, rebuilt_chunks, active) \
             VALUES ($1, $2, 'http://127.0.0.1:1', 'active-model', $3, 0, TRUE) \
             ON CONFLICT (collection_name) DO UPDATE \
             SET fingerprint = EXCLUDED.fingerprint, dimensions = EXCLUDED.dimensions, \
                 active = TRUE, rebuilt_at = NOW()",
        )
        .bind(collection)
        .bind(fingerprint)
        .bind(dimensions)
        .execute(self.db.pool())
        .await
        .expect("seed active vector state");
    }

    pub async fn clear_active_vector_state(&self) {
        sqlx::query("UPDATE context69.vector_index_state SET active = FALSE WHERE active")
            .execute(self.db.pool())
            .await
            .expect("clear the active vector state");
    }

    /// A stored translation of a document, with explicit chunk ids.
    ///
    /// The version row keeps the source hash it was translated from, so a
    /// translation of an older hash stays stored beside the current one.
    pub async fn seed_translation(
        &self,
        document_id: i64,
        target_locale: &str,
        source_record_hash: &str,
        chunks: &[(Uuid, String)],
    ) -> Vec<Uuid> {
        let version_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.document_translation_versions \
             (id, document_id, target_locale, source_record_hash, provider_key, \
              provider_config_hash, translated_title, translated_body_text) \
             VALUES ($1, $2, $3, $4, 'test', 'test', 'test', 'test')",
        )
        .bind(version_id)
        .bind(document_id)
        .bind(target_locale)
        .bind(source_record_hash)
        .execute(self.db.pool())
        .await
        .expect("seed translation version");
        for (index, (chunk_id, text)) in chunks.iter().enumerate() {
            sqlx::query(
                "INSERT INTO context69.document_translation_chunks \
                 (id, translation_id, document_id, target_locale, chunk_index, chunk_text) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(chunk_id)
            .bind(version_id)
            .bind(document_id)
            .bind(target_locale)
            .bind(i32::try_from(index).expect("chunk index"))
            .bind(text)
            .execute(self.db.pool())
            .await
            .expect("seed translation chunk");
        }
        chunks.iter().map(|(chunk_id, _)| *chunk_id).collect()
    }

    /// The stored translation versions of a document.
    pub async fn translation_version_count(&self, document_id: i64) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM context69.document_translation_versions WHERE document_id = $1",
        )
        .bind(document_id)
        .fetch_one(self.db.pool())
        .await
        .expect("count translation versions")
    }

    /// The registry entries that hold a point id in their retired set.
    pub async fn registry_tombstones(
        &self,
        point_id: Uuid,
    ) -> Vec<context69_search::SearchRegistryEntry> {
        self.db
            .read_search_registry_state()
            .await
            .expect("read registry state")
            .entries
            .into_iter()
            .filter(|entry| entry.retired_ids.contains(&point_id))
            .collect()
    }

    pub async fn delete_current_version(&self, document_id: i64, record_hash: &str) {
        sqlx::query(
            "DELETE FROM context69.document_versions WHERE document_id = $1 AND record_hash = $2",
        )
        .bind(document_id)
        .bind(record_hash)
        .execute(self.db.pool())
        .await
        .expect("delete current version");
    }

    pub async fn document_hash(&self, document_id: i64) -> String {
        sqlx::query_scalar("SELECT record_hash FROM context69.documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(self.db.pool())
            .await
            .expect("read document hash")
    }

    pub async fn document_metadata(&self, document_id: i64) -> serde_json::Value {
        sqlx::query_scalar("SELECT metadata_json FROM context69.documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(self.db.pool())
            .await
            .expect("read document metadata")
    }

    pub async fn version_body(&self, document_id: i64, record_hash: &str) -> Option<String> {
        sqlx::query_scalar(
            "SELECT body_text FROM context69.document_versions \
             WHERE document_id = $1 AND record_hash = $2",
        )
        .bind(document_id)
        .bind(record_hash)
        .fetch_optional(self.db.pool())
        .await
        .expect("read version body")
    }

    pub async fn version_count(&self, document_id: i64) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM context69.document_versions WHERE document_id = $1",
        )
        .bind(document_id)
        .fetch_one(self.db.pool())
        .await
        .expect("count versions")
    }

    pub async fn file_record(&self) -> context69::domain::LibraryFileRecord {
        self.store
            .get_file(self.file_id)
            .await
            .expect("read file")
            .expect("fixture file exists")
    }

    pub async fn cleanup(self) {
        let _ =
            sqlx::query("DELETE FROM context69.library_file_metadata_mirror WHERE file_id = $1")
                .bind(self.file_id)
                .execute(self.db.pool())
                .await;
        let _ = sqlx::query("DELETE FROM context69.library_files WHERE group_id = $1")
            .bind(self.group.id)
            .execute(self.db.pool())
            .await;
        let _ = sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
            .bind(self.group.id)
            .execute(self.db.pool())
            .await;
        let _ = sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(self.group.id)
            .execute(self.db.pool())
            .await;
    }
}

async fn seed_group(db: &Database) -> GroupSeed {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'private', 'shared', $3) \
         RETURNING id, group_key, full_path",
    )
    .bind(format!("metadata-{}", Uuid::new_v4()))
    .bind("Metadata Atomic Test Group")
    .bind(format!("test/metadata-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    GroupSeed {
        id: row.get("id"),
        key: row.get("group_key"),
        path: row.get("full_path"),
    }
}

// ---------------------------------------------------------------------------
// Mirror fixture (issue #27 P8)
// ---------------------------------------------------------------------------

/// The vector space of the mirror tests.
pub const MIRROR_DIMENSIONS: usize = 4;
pub const MIRROR_EMBEDDING_BASE_URL: &str = "http://127.0.0.1:1";
pub const MIRROR_EMBEDDING_MODEL: &str = "mirror-model";

/// A deterministic provider so the runtime can publish a snapshot.
pub struct MirrorEmbedding;

#[async_trait]
impl context69::embedding::EmbeddingProvider for MirrorEmbedding {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; MIRROR_DIMENSIONS]).collect())
    }
}

/// The fingerprint of a physical collection in the mirror test space.
pub fn mirror_fingerprint(qdrant_url: &str, collection: &str) -> String {
    context69::embedding::embedding_runtime::vector_identity_fingerprint(
        qdrant_url,
        collection,
        MIRROR_EMBEDDING_BASE_URL,
        MIRROR_EMBEDDING_MODEL,
        MIRROR_DIMENSIONS,
    )
}

/// A published runtime for one mirror test target.
pub fn mirror_runtime(index: QdrantIndex, qdrant_url: &str) -> (VectorRuntime, String) {
    let collection = index.collection_name().to_string();
    let fingerprint = mirror_fingerprint(qdrant_url, &collection);
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let generation = runtime.generation();
    assert!(runtime.publish(
        generation,
        VectorSnapshot {
            embedding: Arc::new(MirrorEmbedding),
            identity: Some(context69::embedding::EmbeddingIdentity {
                base_url: MIRROR_EMBEDDING_BASE_URL.to_string(),
                model: MIRROR_EMBEDDING_MODEL.to_string(),
                dimensions: MIRROR_DIMENSIONS,
            }),
            index: Arc::new(index),
            collection,
            generation,
            configured_fingerprint: Some(fingerprint.clone()),
        }
    ));
    (runtime, fingerprint)
}

/// One fixture of the mirror worker tests: a seeded group and file, a desired
/// and active vector identity, the published runtime and the worker under
/// test.
pub struct MirrorFixture {
    pub db: Database,
    pub store: LibraryStore,
    pub service: Arc<LibraryService>,
    pub runtime: VectorRuntime,
    pub worker: MetadataPayloadSyncWorker,
    pub group: GroupSeed,
    pub file_id: Uuid,
    pub collection: String,
    pub fingerprint: String,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl MirrorFixture {
    pub async fn new(index: QdrantIndex, qdrant_url: &str) -> MirrorFixture {
        let guard = SUITE_LOCK.lock().await;
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap library schema");
        reset_barrier(&db).await;
        sqlx::query("DELETE FROM context69.vector_index_state")
            .execute(db.pool())
            .await
            .expect("clear stale vector pointers");
        sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
            .execute(db.pool())
            .await
            .expect("clear stale desired qdrant settings");
        sqlx::query("DELETE FROM context69.runtime_embedding_settings")
            .execute(db.pool())
            .await
            .expect("clear stale desired embedding settings");
        sqlx::query("DELETE FROM context69.library_file_metadata_mirror")
            .execute(db.pool())
            .await
            .expect("clear stale mirror markers");
        let group = seed_group(&db).await;
        let store = LibraryStore::new(db.clone());
        let storage_root = std::env::temp_dir().join(format!("context69-test-{}", Uuid::new_v4()));
        let collection = index.collection_name().to_string();
        let fingerprint = mirror_fingerprint(qdrant_url, &collection);
        let (runtime, _) = mirror_runtime(index, qdrant_url);
        seed_desired_vector_settings(&db, qdrant_url, &collection).await;
        seed_active_vector_state(&db, &collection, &fingerprint).await;
        let settings = SettingsService::new(db.clone());
        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(NoopCallbacks),
            concurrency: 1,
            readiness: Arc::new(NoopCallbacks),
        });
        let service = LibraryService::with_vector_runtime(
            db.clone(),
            runtime.clone(),
            LibraryServiceConfig {
                chunking: ChunkingConfig {
                    max_chars: 1000,
                    overlap_chars: 100,
                },
                file_library: FileLibraryConfig {
                    storage_root,
                    max_upload_size_mb: 1,
                    trusted_proxy_enabled: false,
                    s3: None,
                },
                valkey_url: None,
                embedding_vector_configured: true,
                embedding_vector_configuration_fingerprint: fingerprint.clone(),
            },
            settings,
            translation,
        )
        .await
        .expect("build library service");
        let worker = MetadataPayloadSyncWorker::new(db.clone(), runtime.clone());
        let file = store
            .create_file_in_project(
                group.id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: None,
                    filename: "mirror.txt".into(),
                    media_type: "text/plain".into(),
                    size_bytes: 5,
                    sha256: "0".repeat(64),
                    storage_rel_path: format!("objects/{}", Uuid::new_v4()),
                    storage_object_id: None,
                    delete_source_after_processing: false,
                },
            )
            .await
            .expect("create fixture file");
        MirrorFixture {
            db,
            store,
            service: Arc::new(service),
            runtime,
            worker,
            group,
            file_id: file.id,
            collection,
            fingerprint,
            _guard: guard,
        }
    }

    pub async fn seed_document(
        &self,
        section_key: &str,
        body: &str,
        chunks: &[&str],
    ) -> SeededDocument {
        let external_id = format!("{section_key}-{}", Uuid::new_v4());
        let source = SourceRecord {
            external_id,
            title: "Mirror Document".into(),
            summary: None,
            body_text: body.into(),
            source_uri: "context69://mirror".into(),
            published_at: None,
            updated_at: chrono::Utc::now(),
            metadata_json: serde_json::json!({"seed": "mirror"}),
        };
        let record_hash = normalize_record(source.clone()).record_hash;
        let payload = ChunkPayload {
            chunk_id: Uuid::nil(),
            document_id: 0,
            group_id: self.group.id,
            group_key: self.group.key.clone(),
            group_path: self.group.path.clone(),
            visibility: Visibility::Private,
            source_key: "library".into(),
            external_id: source.external_id.clone(),
            title: source.title.clone(),
            summary: source.summary.clone(),
            source_uri: source.source_uri.clone(),
            published_at: source.published_at,
            updated_at_source: source.updated_at,
            record_hash: record_hash.clone(),
            chunk_index: 0,
            chunk_text: body.to_string(),
            metadata_json: source.metadata_json.clone(),
            content_locale: "original".into(),
            source_locale: None,
            translation_provider: None,
        };
        let upserted = self
            .db
            .upsert_document(&payload)
            .await
            .expect("seed document");
        let mut chunk_ids = Vec::new();
        let mut chunk_texts = Vec::new();
        let mut rows = Vec::new();
        for (index, text) in chunks.iter().enumerate() {
            let chunk_id = Uuid::new_v4();
            chunk_ids.push(chunk_id);
            chunk_texts.push((*text).to_string());
            rows.push(DocumentChunk {
                id: chunk_id,
                document_id: upserted.document_id,
                chunk_index: i32::try_from(index).expect("chunk index"),
                text: (*text).to_string(),
                record_hash: record_hash.clone(),
            });
        }
        self.db
            .insert_document_chunks(upserted.document_id, &record_hash, &rows)
            .await
            .expect("seed document chunks");
        let mut connection = self.db.pool().acquire().await.expect("acquire");
        self.store
            .merge_file_documents_on(
                &mut connection,
                self.file_id,
                &[LibraryFileDocumentRecord {
                    file_id: self.file_id,
                    document_id: upserted.document_id,
                    group_id: self.group.id,
                    visibility: Visibility::Private,
                    section_key: section_key.into(),
                    section_label: section_key.into(),
                    section_external_id: None,
                    section_source_uri: None,
                    section_published_at: None,
                    section_metadata_json: serde_json::json!({}),
                    sort_order: 0,
                }],
            )
            .await
            .expect("link file document");
        SeededDocument {
            document_id: upserted.document_id,
            record_hash,
            chunk_ids,
            chunk_texts,
        }
    }

    pub async fn make_marker_due(&self) {
        sqlx::query("UPDATE context69.library_file_metadata_mirror SET next_attempt_at = now()")
            .execute(self.db.pool())
            .await
            .expect("make the marker due");
    }

    /// The marker of the fixture file: `(version, attempts)`.
    pub async fn marker(&self) -> Option<(i64, i32)> {
        sqlx::query_as::<_, (i64, i32)>(
            "SELECT version, attempts FROM context69.library_file_metadata_mirror \
             WHERE file_id = $1",
        )
        .bind(self.file_id)
        .fetch_optional(self.db.pool())
        .await
        .expect("read the mirror marker")
    }

    pub async fn set_active(&self, collection: &str, fingerprint: &str) {
        seed_active_vector_state(&self.db, collection, fingerprint).await;
    }

    pub async fn set_desired(&self, qdrant_url: &str, collection: &str) {
        seed_desired_vector_settings(&self.db, qdrant_url, collection).await;
    }

    pub async fn desired_model(&self, model: &str) {
        sqlx::query("UPDATE context69.runtime_embedding_settings SET model = $1 WHERE singleton")
            .bind(model)
            .execute(self.db.pool())
            .await
            .expect("drift the desired model");
    }

    /// Whether a fresh, non-pooled session can take this file's mirror lock.
    pub async fn can_take_session_lock(&self, file_id: Uuid) -> bool {
        can_take_session_lock(file_id).await
    }

    pub async fn cleanup(self) {
        let _ =
            sqlx::query("DELETE FROM context69.library_file_metadata_mirror WHERE file_id = $1")
                .bind(self.file_id)
                .execute(self.db.pool())
                .await;
        let _ = sqlx::query("DELETE FROM context69.library_files WHERE group_id = $1")
            .bind(self.group.id)
            .execute(self.db.pool())
            .await;
        let _ = sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
            .bind(self.group.id)
            .execute(self.db.pool())
            .await;
        let _ = sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(self.group.id)
            .execute(self.db.pool())
            .await;
    }
}

async fn reset_barrier(db: &Database) {
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
             rebuild_fingerprint = NULL, rebuild_collection = NULL, \
             rebuild_started_at = NULL, rebuild_error = NULL \
         WHERE id = 1",
    )
    .execute(db.pool())
    .await
    .expect("reset the fixture barrier");
}

async fn seed_desired_vector_settings(db: &Database, qdrant_url: &str, collection: &str) {
    sqlx::query(
        "INSERT INTO context69.runtime_qdrant_settings \
         (singleton, url, collection_name) VALUES (TRUE, $1, $2) \
         ON CONFLICT (singleton) DO UPDATE \
         SET url = EXCLUDED.url, collection_name = EXCLUDED.collection_name",
    )
    .bind(qdrant_url)
    .bind(collection)
    .execute(db.pool())
    .await
    .expect("seed desired qdrant settings");
    sqlx::query(
        "INSERT INTO context69.runtime_embedding_settings \
         (singleton, base_url, model, dimensions, timeout_secs) \
         VALUES (TRUE, $1, $2, $3, 30) \
         ON CONFLICT (singleton) DO UPDATE \
         SET base_url = EXCLUDED.base_url, model = EXCLUDED.model, \
             dimensions = EXCLUDED.dimensions",
    )
    .bind(MIRROR_EMBEDDING_BASE_URL)
    .bind(MIRROR_EMBEDDING_MODEL)
    .bind(MIRROR_DIMENSIONS as i64)
    .execute(db.pool())
    .await
    .expect("seed desired embedding settings");
}

async fn seed_active_vector_state(db: &Database, collection: &str, fingerprint: &str) {
    sqlx::query("UPDATE context69.vector_index_state SET active = FALSE WHERE active")
        .execute(db.pool())
        .await
        .expect("clear the previous active pointer");
    sqlx::query(
        "INSERT INTO context69.vector_index_state \
         (collection_name, fingerprint, embedding_base_url, embedding_model, \
          dimensions, rebuilt_chunks, active) \
         VALUES ($1, $2, $3, $4, $5, 0, TRUE) \
         ON CONFLICT (collection_name) DO UPDATE \
         SET fingerprint = EXCLUDED.fingerprint, dimensions = EXCLUDED.dimensions, \
             active = TRUE, rebuilt_at = NOW()",
    )
    .bind(collection)
    .bind(fingerprint)
    .bind(MIRROR_EMBEDDING_BASE_URL)
    .bind(MIRROR_EMBEDDING_MODEL)
    .bind(MIRROR_DIMENSIONS as i64)
    .execute(db.pool())
    .await
    .expect("seed active vector state");
}
