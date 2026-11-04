//! The fixture of the atomic first insert regressions (issue #1138 P22).
//!
//! One fixture owns a claimed task item of a fresh group, without a library
//! file, so a case can create that file through the atomic first insert and
//! observe what the transaction committed. Every case changes exactly one
//! thing, so a refused creation can be compared with the state it refused to
//! create.

use cloudiful_tasks::Failure;
use context69::db::{ClaimedItem, CreateTaskSubmissionRequest, Database};
use context69::domain::{ChunkPayload, DocumentChunk};
use context69::library_store::{LibraryStore, NewLibraryFile, TaskWriteFence};
use context69_contracts::Visibility;
use futures::FutureExt;
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
    sibling_items: Vec<ClaimedItem>,
    sibling_tasks: Vec<Uuid>,
    extra_grants: Vec<ClaimedItem>,
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
        let mut extra_grants = Vec::new();
        for _ in 0..40 {
            let returned = db.claim_items_fast().await.expect("claim items");
            assert_owned_claims(&returned, task_id, &items);
            for candidate in returned {
                if candidate.id == items[0] {
                    item = Some(candidate);
                } else {
                    extra_grants.push(candidate);
                }
            }
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
            sibling_items: Vec::new(),
            sibling_tasks: Vec::new(),
            extra_grants,
        }
    }

    /// A claimed sibling item of the same group.
    ///
    /// Two items of one group are the shape of a concurrent writer: the second
    /// one finds the identity the first one committed.
    pub async fn claim_sibling_item(&mut self, payload: serde_json::Value) -> ClaimedItem {
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
            let returned = self.db.claim_items_fast().await.expect("claim items");
            assert_owned_claims(&returned, task_id, &items);
            let mut selected = None;
            for candidate in returned {
                if candidate.id == items[0] {
                    selected = Some(candidate);
                } else {
                    self.extra_grants.push(candidate);
                }
            }
            if let Some(item) = selected {
                self.sibling_tasks.push(task_id);
                self.sibling_items.push(item);
                return self
                    .sibling_items
                    .last()
                    .expect("the sibling was just retained")
                    .clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the sibling item is claimable");
    }

    /// Await the fixture teardown on both success and panic, then propagate the
    /// original outcome. The teardown seals every captured grant and deletes
    /// only this fixture's task rows and user; groups and library files are
    /// unrelated state left untouched.
    pub async fn finish<T>(mut self, outcome: Result<T, Box<dyn std::any::Any + Send>>) -> T {
        let errors = self.release().await;
        match outcome {
            Ok(value) => {
                assert!(
                    errors.is_empty(),
                    "first-insert fixture teardown failed: {}",
                    errors.join("; ")
                );
                value
            }
            Err(payload) => {
                if !errors.is_empty() {
                    eprintln!(
                        "first-insert fixture teardown also failed: {}",
                        errors.join("; ")
                    );
                }
                std::panic::resume_unwind(payload)
            }
        }
    }

    async fn release(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut grants = vec![self.item.clone()];
        grants.extend(self.sibling_items.iter().cloned());
        grants.extend(self.extra_grants.iter().cloned());
        for grant in grants {
            let sealed = std::panic::AssertUnwindSafe(seal_captured_grant(&self.db, &grant))
                .catch_unwind()
                .await;
            if let Err(payload) = sealed {
                let detail = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| {
                        payload
                            .downcast_ref::<&str>()
                            .map(|text| (*text).to_owned())
                    })
                    .unwrap_or_else(|| "non-string panic".to_owned());
                errors.push(format!(
                    "seal grant task_id={} item_id={} panicked: {detail}",
                    grant.task_id, grant.id
                ));
            }
        }
        let mut tasks = vec![self.task_id];
        tasks.extend(self.sibling_tasks.iter().copied());
        for task_id in tasks {
            if let Err(error) = delete_task(&self.db, task_id).await {
                errors.push(error);
            }
        }
        if let Err(error) = delete_user(&self.db, self.user).await {
            errors.push(error);
        }
        errors
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

/// Assert every grant a claim returned belongs to this fixture's known task and
/// item set, so another captured grant is never silently discarded.
fn assert_owned_claims(returned: &[ClaimedItem], task_id: Uuid, item_ids: &[Uuid]) {
    for item in returned {
        assert!(
            item.task_id == task_id && item_ids.contains(&item.id),
            "claim returned a grant outside this fixture: \
             task_id={} item_id={} fixture_task={task_id} fixture_items={item_ids:?}",
            item.task_id,
            item.id
        );
    }
}

/// Seal the exact captured library attempt through the scoped queue API. An
/// already sealed grant is confirmed from the scoped attempt history;
/// `LeaseLost` alone is never treated as expiry or terminal state, and no grant
/// is forged or renewed.
async fn seal_captured_grant(db: &Database, item: &ClaimedItem) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let queue = db
        .execution_queue()
        .await
        .expect("load the execution queue");
    let result = queue
        .fail(
            &execution.attempt,
            &Failure::permanent(Some(
                serde_json::json!({"reason": "context69_test_cleanup"}),
            )),
        )
        .await;
    match result {
        Ok(_) => {}
        Err(
            error @ (cloudiful_tasks::Error::LeaseLost { .. }
            | cloudiful_tasks::Error::InvalidTaskState { .. }),
        ) => {
            let attempts = queue
                .attempts(execution.attempt.task_id(), 16)
                .await
                .expect("read the captured task attempts");
            let sealed = attempts.iter().any(|attempt| {
                attempt.id() == execution.attempt.id() && attempt.status().is_sealed()
            });
            assert!(
                sealed,
                "fixture teardown must seal the captured attempt or confirm it already sealed: {error}"
            );
        }
        Err(error) => panic!("fixture teardown could not seal the captured attempt: {error}"),
    }
}

async fn delete_task(db: &Database, task_id: Uuid) -> Result<(), String> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete task items {task_id}: {error}"))?;
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("delete task {task_id}: {error}"))?;
    Ok(())
}

async fn delete_user(db: &Database, user_id: i64) -> Result<(), String> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete idempotency keys for user {user_id}: {error}"))?;
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("delete user {user_id}: {error}"))?;
    Ok(())
}
