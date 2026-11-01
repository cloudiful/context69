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
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::settings::SettingsService;
use context69_contracts::Visibility;
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use sqlx::Row;
use uuid::Uuid;

pub const CANONICAL_BODY: &str = "alpha beta gamma delta epsilon";

struct NoopCallbacks;

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
}

impl MetadataFixture {
    pub async fn new() -> MetadataFixture {
        MetadataFixture::new_with_caps(None).await
    }

    /// The fixture with explicit search-registry admission caps.
    ///
    /// Without the caps the metadata publisher keeps its disabled behavior: it
    /// changes the file and its documents and retires no translation point.
    pub async fn new_with_caps(caps: Option<[i64; 3]>) -> MetadataFixture {
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap library schema");
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
                    max_upload_request_size_mb: 1,
                    ingest_concurrency: 1,
                    url_import_concurrency: 1,
                    url_import_min_interval_ms: 1000,
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
