//! The fixture of the atomic first insert regressions (issue #1138 P22).
//!
//! One fixture owns a claimed task item of a fresh group, without a library
//! file, so a case can create that file through the atomic first insert and
//! observe what the transaction committed. Every case changes exactly one
//! thing, so a refused creation can be compared with the state it refused to
//! create.

use context69::db::{ClaimedItem, CreateTaskSubmissionRequest, Database};
use context69::domain::{ChunkPayload, DocumentChunk};
use context69::library_store::{LibraryStore, NewLibraryFile, TaskWriteFence};
use context69_contracts::Visibility;
use uuid::Uuid;

/// One claimed task item of a group that owns no library file yet.
pub struct FirstInsertFixture {
    pub db: Database,
    pub store: LibraryStore,
    pub group: i64,
    pub user: i64,
    pub task_id: Uuid,
    pub item: ClaimedItem,
    pub fence: TaskWriteFence,
}

impl FirstInsertFixture {
    /// Claim one item of a fresh group for the given worker payload.
    pub async fn new(payload: serde_json::Value) -> FirstInsertFixture {
        Self::new_with_kind("text_batch", payload).await
    }

    /// Claim one item of a fresh group for the given task kind and payload.
    pub async fn new_with_kind(kind: &str, payload: serde_json::Value) -> FirstInsertFixture {
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap the library schema");
        let user = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_user.sql",
            format!("first-insert-{}", Uuid::new_v4()),
            "First Insert",
            "unused"
        )
        .fetch_one(db.pool())
        .await
        .expect("seed user");
        let group = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_group.sql",
            format!("first-insert-{}", Uuid::new_v4()),
            "First Insert",
            format!("test/first-insert-{}", Uuid::new_v4())
        )
        .fetch_one(db.pool())
        .await
        .expect("seed group");
        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(group),
                kind,
                group_path: Some("test/first-insert"),
                source_key: None,
                payloads: &[payload],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("first-insert-{}", Uuid::new_v4()),
            })
            .await
            .expect("create task");
        let mut item = None;
        for _ in 0..40 {
            item = db
                .claim_items_fast(500)
                .await
                .expect("claim items")
                .into_iter()
                .find(|candidate| candidate.id == items[0]);
            if item.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let item = item.expect("the fixture item is claimable");
        let fence = TaskWriteFence::from_claimed_item(&item).expect("a task write fence");
        let store = LibraryStore::new(db.clone());
        FirstInsertFixture {
            db,
            store,
            group,
            user,
            task_id,
            item,
            fence,
        }
    }

    /// A claimed sibling item of the same group.
    ///
    /// Two items of one group are the shape of a concurrent writer: the second
    /// one finds the identity the first one committed.
    pub async fn claim_sibling_item(&self, payload: serde_json::Value) -> ClaimedItem {
        let task_id = Uuid::new_v4();
        let (_, _, items) = self
            .db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: self.user,
                group_id: Some(self.group),
                kind: "text_batch",
                group_path: Some("test/first-insert"),
                source_key: None,
                payloads: &[payload],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("first-insert-{}", Uuid::new_v4()),
            })
            .await
            .expect("create the sibling task");
        for _ in 0..40 {
            let item = self
                .db
                .claim_items_fast(500)
                .await
                .expect("claim items")
                .into_iter()
                .find(|candidate| candidate.id == items[0]);
            if let Some(item) = item {
                return item;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the sibling item is claimable");
    }

    /// A storage object of the group that no task item references.
    pub async fn create_object(&self, sha256: &str, size_bytes: i64) -> Uuid {
        let object = Uuid::new_v4();
        let key = format!("objects/{object}");
        sqlx::query_file!(
            "tests/library_ingest_switch/insert_object.sql",
            object,
            self.group,
            sha256,
            size_bytes,
            key
        )
        .execute(self.db.pool())
        .await
        .expect("insert storage object");
        object
    }

    /// A storage object of another group.
    ///
    /// The atomic first insert refuses a staged object of another group, so
    /// the case can prove that the refusal happens inside the transaction that
    /// creates the file.
    pub async fn create_foreign_object(&self, sha256: &str, size_bytes: i64) -> Uuid {
        let group = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_group.sql",
            format!("first-insert-foreign-{}", Uuid::new_v4()),
            "First Insert Foreign",
            format!("test/first-insert-foreign-{}", Uuid::new_v4())
        )
        .fetch_one(self.db.pool())
        .await
        .expect("seed the foreign group");
        let object = Uuid::new_v4();
        let key = format!("objects/{object}");
        sqlx::query_file!(
            "tests/library_ingest_switch/insert_object.sql",
            object,
            group,
            sha256,
            size_bytes,
            key
        )
        .execute(self.db.pool())
        .await
        .expect("insert the foreign storage object");
        object
    }

    /// A committed file of the group that was never ingested.
    pub async fn create_failed_file(&self, external_id: &str) -> Uuid {
        let file = self
            .store
            .create_file_in_project(
                self.group,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: Some(external_id.to_string()),
                    filename: format!("{external_id}.txt"),
                    media_type: "text/plain".into(),
                    size_bytes: 4,
                    sha256: "0".repeat(64),
                    storage_rel_path: format!("objects/{}", Uuid::new_v4()),
                    storage_object_id: None,
                    delete_source_after_processing: false,
                },
            )
            .await
            .expect("create the failed file");
        file.id
    }

    /// A document with one chunk, linked to the given file.
    pub async fn seed_document(&self, file_id: Uuid, section_key: &str) -> (i64, Uuid) {
        let chunk_id = Uuid::new_v4();
        let payload = ChunkPayload {
            chunk_id,
            document_id: 0,
            group_id: self.group,
            group_key: format!("first-insert-{}", self.group),
            group_path: format!("test/first-insert-{}", self.group),
            visibility: Visibility::Private,
            source_key: "library".into(),
            external_id: format!("{section_key}-{}", Uuid::new_v4()),
            title: "test".into(),
            summary: None,
            source_uri: "context69://test".into(),
            published_at: None,
            updated_at_source: chrono::Utc::now(),
            record_hash: "old-hash".into(),
            chunk_index: 0,
            chunk_text: "old".into(),
            metadata_json: serde_json::json!({}),
            content_locale: "original".into(),
            source_locale: None,
            translation_provider: None,
        };
        let upserted = self
            .db
            .upsert_document(&payload)
            .await
            .expect("upsert document");
        let chunk = DocumentChunk {
            id: chunk_id,
            document_id: upserted.document_id,
            chunk_index: 0,
            text: "old".into(),
            record_hash: "old-hash".into(),
        };
        self.db
            .insert_document_chunks(upserted.document_id, "old-hash", &[chunk])
            .await
            .expect("insert document chunk");
        let mut connection = self.db.pool().acquire().await.expect("acquire");
        self.store
            .merge_file_documents_on(
                &mut connection,
                file_id,
                &[context69::domain::LibraryFileDocumentRecord {
                    file_id,
                    document_id: upserted.document_id,
                    group_id: self.group,
                    visibility: Visibility::Private,
                    section_key: section_key.to_string(),
                    section_label: section_key.to_string(),
                    section_external_id: None,
                    section_source_uri: None,
                    section_published_at: None,
                    section_metadata_json: serde_json::json!({}),
                    sort_order: 0,
                }],
            )
            .await
            .expect("link the document");
        (upserted.document_id, chunk_id)
    }
}
