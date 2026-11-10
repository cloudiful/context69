//! Fixture of the atomic library delete tests (issue #1138 P25).
//!
//! One fixture owns a group, the storage root and one `LibraryService` built on
//! a published vector runtime. The seeded rows are namespaced by a unique id and
//! removed again when the test finishes: an entity tombstone is global state by
//! design, so the cleanup is part of the fixture instead of an afterthought.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::{ClaimedItem, CreateTaskSubmissionRequest, Database};
use context69::domain::GroupRecord;
use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
use context69::library_store::TaskWriteFence;
use context69::qdrant_index::QdrantIndex;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::settings::SettingsService;
use context69::vector_runtime::{VectorRuntime, VectorSnapshot};
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

pub const DIMENSIONS: usize = 4;
pub const HARNESS_EMBEDDING_BASE_URL: &str = "http://127.0.0.1:1";
pub const HARNESS_EMBEDDING_MODEL: &str = "harness";

/// The deployed fingerprint of one collection in this fixture's test space.
pub fn harness_fingerprint(qdrant_url: &str, collection: &str) -> String {
    context69::embedding::embedding_runtime::vector_identity_fingerprint(
        qdrant_url,
        collection,
        HARNESS_EMBEDDING_BASE_URL,
        HARNESS_EMBEDDING_MODEL,
        DIMENSIONS,
    )
}

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

/// A deterministic embedding provider so the runtime can publish a snapshot.
struct FakeEmbedding;

#[async_trait]
impl EmbeddingProvider for FakeEmbedding {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| vec![text.len() as f32; DIMENSIONS])
            .collect())
    }
}

/// One claimed task item of the fixture.
pub struct TaskFixture {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub fence: TaskWriteFence,
    pub claimed: ClaimedItem,
}

/// The group, the storage root and the service of one test.
pub struct DeleteFixture {
    pub db: Database,
    pub library: LibraryService,
    pub group: GroupRecord,
    pub storage_root: PathBuf,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl DeleteFixture {
    /// Build a fixture on a service with the given index published.
    pub async fn new(index: Option<QdrantIndex>, qdrant_url: &str) -> Self {
        let guard = SUITE_LOCK.lock().await;
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap the library schema");
        let storage_root = std::env::temp_dir().join(format!("context69-p25-{}", Uuid::new_v4()));
        let group = seed_group(&db).await;
        let library = build_library(&db, &storage_root, index, qdrant_url, true).await;
        Self {
            library,
            db,
            group,
            storage_root,
            _guard: guard,
        }
    }

    /// Build one more service on the same database and storage root.
    pub async fn library_with_index(
        &self,
        index: Option<QdrantIndex>,
        qdrant_url: &str,
    ) -> LibraryService {
        build_library(&self.db, &self.storage_root, index, qdrant_url, true).await
    }

    /// Build a service whose published pair carries no identity fingerprint
    /// while the deployment still has a desired and an active identity.
    pub async fn library_without_identity(
        &self,
        index: Option<QdrantIndex>,
        qdrant_url: &str,
    ) -> LibraryService {
        build_library(&self.db, &self.storage_root, index, qdrant_url, false).await
    }

    /// Seed one library file row without documents, in the given folder.
    pub async fn seed_file(&self, folder_id: Option<Uuid>) -> Uuid {
        let file_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.library_files \
             (id, group_id, visibility, folder_id, external_id, filename, media_type, \
              size_bytes, sha256, storage_rel_path, ingest_status) \
             VALUES ($1, $2, 'public', $3, $4, $5, 'text/plain', 4, $6, $7, 'succeeded')",
        )
        .bind(file_id)
        .bind(self.group.id)
        .bind(folder_id)
        .bind(format!("p25-{file_id}"))
        .bind(format!("p25-{file_id}.txt"))
        .bind(format!("{:064x}", file_id.as_u128()))
        .bind(format!("p25/{}", file_id))
        .execute(self.db.pool())
        .await
        .expect("seed library file");
        file_id
    }

    /// Link one document with one chunk to the file and return the document id.
    pub async fn seed_document(&self, file_id: Uuid, section_key: &str) -> i64 {
        let (document_id, _) = self
            .seed_plain_document(&format!("file_library_{section_key}"))
            .await;
        let chunk_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.document_chunks \
             (id, document_id, chunk_index, chunk_text, record_hash) \
             VALUES ($1, $2, 0, 'p25 chunk', $3)",
        )
        .bind(chunk_id)
        .bind(document_id)
        .bind(format!("p25-hash-{document_id}"))
        .execute(self.db.pool())
        .await
        .expect("seed document chunk");
        sqlx::query(
            "INSERT INTO context69.library_file_documents \
             (file_id, document_id, group_id, visibility, section_key, section_label, \
              section_metadata_json, sort_order) \
             VALUES ($1, $2, $3, 'public', $4, $4, '{}'::jsonb, 0)",
        )
        .bind(file_id)
        .bind(document_id)
        .bind(self.group.id)
        .bind(section_key)
        .execute(self.db.pool())
        .await
        .expect("link the document to the file");
        document_id
    }

    /// Seed one document that no library file owns.
    pub async fn seed_plain_document(&self, external_id: &str) -> (i64, String) {
        let external_id = format!("{external_id}-{}", Uuid::new_v4());
        let document_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO context69.documents \
             (source_key, external_id, title, source_uri, updated_at_source, record_hash, \
              group_id, visibility, metadata_json) \
             VALUES ('p25', $1, 'p25', 'context69://p25', now(), $2, $3, 'public', \
              '{}'::jsonb) \
             RETURNING id",
        )
        .bind(&external_id)
        .bind(format!("p25-record-{}", Uuid::new_v4()))
        .bind(self.group.id)
        .fetch_one(self.db.pool())
        .await
        .expect("seed a plain document");
        (document_id, external_id)
    }

    /// Seed one folder below `parent`.
    pub async fn seed_folder(&self, parent: Option<Uuid>) -> Uuid {
        let folder_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.library_folders \
             (id, group_id, visibility, parent_id, name) \
             VALUES ($1, $2, 'public', $3, $4)",
        )
        .bind(folder_id)
        .bind(self.group.id)
        .bind(parent)
        .bind(format!("p25-{}", folder_id))
        .execute(self.db.pool())
        .await
        .expect("seed library folder");
        folder_id
    }

    /// Seed a user, a task and one claimed item of it.
    pub async fn seed_task(&self) -> TaskFixture {
        let user = sqlx::query_scalar::<_, i64>(
            "INSERT INTO context69.users (login_name, display_name, password_hash) \
             VALUES ($1, 'P25 Delete', 'unused') RETURNING id",
        )
        .bind(format!("p25-delete-{}", Uuid::new_v4()))
        .fetch_one(self.db.pool())
        .await
        .expect("seed user");
        sqlx::query(
            "INSERT INTO context69.group_memberships (group_id, user_id, role) \
             VALUES ($1, $2, 'owner')",
        )
        .bind(self.group.id)
        .bind(user)
        .execute(self.db.pool())
        .await
        .expect("seed group membership");
        let task_id = Uuid::new_v4();
        let (_, _, items) = self
            .db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(self.group.id),
                kind: "text_batch",
                group_path: Some("test/p25"),
                source_key: None,
                payloads: &[json!({"external_id": "p25"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("p25-{}", Uuid::new_v4()),
            })
            .await
            .expect("create task");
        let mut claimed = None;
        for _ in 0..40 {
            claimed = self
                .db
                .claim_items_fast(500)
                .await
                .expect("claim items")
                .into_iter()
                .find(|item| item.id == items[0]);
            if claimed.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let claimed = claimed.expect("the fixture item is claimable");
        let fence = TaskWriteFence::from_claimed_item(&claimed).expect("the captured grant");
        TaskFixture {
            task_id,
            item_id: claimed.id,
            fence,
            claimed,
        }
    }

    /// Crash the worker that holds the item: the library attempt and the
    /// domain mirror expire together, so the captured grant is gone.
    pub async fn crash(&self, task: &TaskFixture) {
        let execution = task
            .claimed
            .execution_context()
            .expect("the claimed item carries its execution");
        sqlx::query(
            "UPDATE cloudiful_tasks.attempts SET lease_expires_at = now() - interval '1 minute' \
             WHERE id = $1",
        )
        .bind(execution.attempt_id())
        .execute(self.db.pool())
        .await
        .expect("expire the library attempt");
        sqlx::query(
            "UPDATE context69.task_items SET lease_until = now() - interval '1 minute' WHERE id = $1",
        )
        .bind(task.item_id)
        .execute(self.db.pool())
        .await
        .expect("expire the domain mirror");
    }

    /// Seed one active unresolved ingest run of the file.
    pub async fn seed_active_ingest_run(&self, file_id: Uuid, task: &TaskFixture) {
        sqlx::query(
            "INSERT INTO context69.library_ingest_runs \
             (item_id, task_id, file_id, batch_id, generation, mode, source, lease_token, \
              execution_task_id, execution_generation) \
             SELECT $1, $2, $3, $4, nextval('context69.library_ingest_generation_seq'), \
                    'first_insert', 'stored', $5, $6, $7",
        )
        .bind(task.item_id)
        .bind(task.task_id)
        .bind(file_id)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(0_i64)
        .execute(self.db.pool())
        .await
        .expect("seed the active ingest run");
    }

    /// Whether a file row still exists.
    pub async fn file_exists(&self, file_id: Uuid) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM context69.library_files WHERE id = $1")
            .bind(file_id)
            .fetch_one(self.db.pool())
            .await
            .expect("count the file")
            > 0
    }

    /// The number of documents of one file.
    pub async fn file_document_count(&self, file_id: Uuid) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM context69.documents d \
             JOIN context69.library_file_documents l ON l.document_id = d.id \
             WHERE l.file_id = $1",
        )
        .bind(file_id)
        .fetch_one(self.db.pool())
        .await
        .expect("count the documents")
    }

    /// The ids of the entity tombstones of the given files and documents.
    pub async fn tombstone_ids(&self, file_ids: &[Uuid], document_ids: &[i64]) -> Vec<i64> {
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM context69.library_deleted_entities \
             WHERE file_id = ANY($1) OR document_id = ANY($2) ORDER BY id",
        )
        .bind(file_ids)
        .bind(document_ids)
        .fetch_all(self.db.pool())
        .await
        .expect("read the tombstone ids")
    }

    /// Remove the tombstones of one test by their exact ids.
    pub async fn clear_tombstones(&self, ids: &[i64]) {
        if ids.is_empty() {
            return;
        }
        sqlx::query("DELETE FROM context69.library_deleted_entities WHERE id = ANY($1)")
            .bind(ids)
            .execute(self.db.pool())
            .await
            .expect("clear the tombstones");
    }

    /// Remove every row the fixture created.
    pub async fn cleanup(self) {
        let _ = std::fs::remove_dir_all(&self.storage_root);
        let _ = sqlx::query("UPDATE context69.vector_index_state SET active = FALSE WHERE active")
            .execute(self.db.pool())
            .await;
        let _ = sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
            .execute(self.db.pool())
            .await;
        let _ = sqlx::query("DELETE FROM context69.runtime_embedding_settings")
            .execute(self.db.pool())
            .await;
        for statement in [
            "DELETE FROM context69.library_files WHERE group_id = $1",
            "DELETE FROM context69.library_folders WHERE group_id = $1",
            "DELETE FROM context69.library_storage_object_cleanup WHERE group_id = $1",
            "DELETE FROM context69.documents WHERE group_id = $1",
            "DELETE FROM context69.groups WHERE id = $1",
        ] {
            let _ = sqlx::query(statement)
                .bind(self.group.id)
                .execute(self.db.pool())
                .await;
        }
    }
}

/// Build one library service on a database and storage root.
async fn build_library(
    db: &Database,
    storage_root: &std::path::Path,
    index: Option<QdrantIndex>,
    qdrant_url: &str,
    publish_fingerprint: bool,
) -> LibraryService {
    let embedding = Arc::new(FakeEmbedding);
    let configured = index.is_some();
    let runtime = VectorRuntime::from_legacy(
        EmbeddingRuntime::new(Some(embedding.clone()), None),
        None,
        "",
    );
    if let Some(index) = index {
        let collection = index.collection_name().to_string();
        let fingerprint = harness_fingerprint(qdrant_url, &collection);
        seed_desired_vector_settings(db, qdrant_url, &collection).await;
        seed_active_vector_state(db, &collection, &fingerprint).await;
        let generation = runtime.generation();
        assert!(runtime.publish(
            generation,
            VectorSnapshot {
                embedding,
                identity: None,
                index: Arc::new(index),
                collection: collection.clone(),
                generation,
                configured_fingerprint: publish_fingerprint.then_some(fingerprint),
            }
        ));
    } else {
        clear_desired_vector_settings(db).await;
        clear_active_vector_state(db).await;
    }
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    LibraryService::with_vector_runtime(
        db.clone(),
        runtime,
        LibraryServiceConfig {
            chunking: ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            file_library: FileLibraryConfig {
                storage_root: storage_root.to_path_buf(),
                max_upload_size_mb: 1,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: configured,
            embedding_vector_configuration_fingerprint: "p25".to_string(),
        },
        SettingsService::new(db.clone()),
        translation,
    )
    .await
    .expect("build library service")
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
    .bind(HARNESS_EMBEDDING_BASE_URL)
    .bind(HARNESS_EMBEDDING_MODEL)
    .bind(DIMENSIONS as i64)
    .execute(db.pool())
    .await
    .expect("seed desired embedding settings");
}

async fn clear_desired_vector_settings(db: &Database) {
    sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
        .execute(db.pool())
        .await
        .expect("clear the desired qdrant settings");
    sqlx::query("DELETE FROM context69.runtime_embedding_settings")
        .execute(db.pool())
        .await
        .expect("clear the desired embedding settings");
}

async fn seed_active_vector_state(db: &Database, collection: &str, fingerprint: &str) {
    sqlx::query("UPDATE context69.vector_index_state SET active = FALSE WHERE active")
        .execute(db.pool())
        .await
        .expect("claim the active vector index");
    sqlx::query(
        "INSERT INTO context69.vector_index_state (
             collection_name, fingerprint, embedding_base_url, embedding_model,
             dimensions, rebuilt_chunks, active
         )
         VALUES ($1, $2, $3, $4, $5, 0, TRUE)
         ON CONFLICT (collection_name) DO UPDATE
         SET fingerprint = EXCLUDED.fingerprint,
             embedding_base_url = EXCLUDED.embedding_base_url,
             embedding_model = EXCLUDED.embedding_model,
             dimensions = EXCLUDED.dimensions,
             active = TRUE,
             rebuilt_at = NOW()",
    )
    .bind(collection)
    .bind(fingerprint)
    .bind(HARNESS_EMBEDDING_BASE_URL)
    .bind(HARNESS_EMBEDDING_MODEL)
    .bind(DIMENSIONS as i64)
    .execute(db.pool())
    .await
    .expect("seed the active vector index");
}

async fn clear_active_vector_state(db: &Database) {
    sqlx::query("UPDATE context69.vector_index_state SET active = FALSE WHERE active")
        .execute(db.pool())
        .await
        .expect("clear the active vector index");
}

async fn seed_group(db: &Database) -> GroupRecord {
    let key = format!("p25-delete-{}", Uuid::new_v4());
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $1, 'public', 'shared', $1) \
         RETURNING id, group_key, name, full_path, created_at, updated_at",
    )
    .bind(&key)
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    GroupRecord {
        id: row.get("id"),
        parent_group_id: None,
        group_key: row.get("group_key"),
        group_path: row.get("full_path"),
        parent_group_path: None,
        name: row.get("name"),
        visibility: context69::contracts::Visibility::Public,
        kind: context69::contracts::GroupKind::Shared,
        owner_user_id: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        current_role: None,
    }
}
