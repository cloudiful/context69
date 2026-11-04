//! The fixture of the library replacement switch regression tests.
//!
//! One fixture owns a claimed task item, a library file with a live source
//! object, the durable ingest run of a replacement and the registry entry that
//! admits the staged point set. Every test starts from that state and then
//! changes exactly one thing, so a refused switch can be compared with the
//! state it refused to change.

use context69::db::{
    CreateTaskSubmissionRequest, Database, ExecutionContext, IngestRunMode, IngestRunSource,
    NewIngestRun, RegistryQuotaOverride,
};
use context69::domain::{ChunkPayload, DocumentChunk, LibraryFileDocumentRecord};
use context69::library_store::{LibraryStore, NewLibraryFile, TaskWriteFence, begin_task_write};
use context69::services::library::{
    PreparedReplacement, PreparedReplacementSection, ReplacementDocumentBaseline,
    ReplacementFileBaseline,
};
use context69_contracts::Visibility;
use context69_search::SearchRegistryEntry;
use uuid::Uuid;

/// One seeded section of the library file.
pub struct SeededSection {
    pub document_id: i64,
    pub chunk_id: Uuid,
    pub external_id: String,
    pub record_hash: String,
}

/// The source and status of the library file.
#[derive(Debug, sqlx::FromRow)]
pub struct FileSourceRow {
    pub storage_object_id: Option<Uuid>,
    pub storage_rel_path: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub ingest_status: String,
}

/// The claimed task item, the library file and the durable run of a replacement.
pub struct SwitchFixture {
    pub db: Database,
    pub store: LibraryStore,
    pub group: i64,
    pub user: i64,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub file_id: Uuid,
    pub old_object: Uuid,
    pub fence: TaskWriteFence,
    execution: ExecutionContext,
}

impl SwitchFixture {
    pub async fn new() -> SwitchFixture {
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .unwrap();
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .unwrap();
        // The fixture supplies its own captured vector identity to the runner.
        // The shared test database may still hold the persisted identity of
        // another target, so it is removed here: a case must not be refused
        // because a target that ran earlier configured another vector space.
        sqlx::query("DELETE FROM context69.runtime_embedding_settings WHERE singleton = TRUE")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM context69.runtime_qdrant_settings WHERE singleton = TRUE")
            .execute(db.pool())
            .await
            .unwrap();
        // A rebuild barrier left by another target would refuse the fixture's
        // admission before it reaches its own subject. The fixture owns its
        // schema state, so it restores the barrier to idle first.
        sqlx::query(
            "UPDATE context69.search_registry_epoch \
             SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
                 rebuild_fingerprint = NULL, rebuild_collection = NULL, \
                 rebuild_started_at = NULL, rebuild_error = NULL \
             WHERE id = 1",
        )
        .execute(db.pool())
        .await
        .unwrap();
        let user = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_user.sql",
            format!("switch-{}", Uuid::new_v4()),
            "Switch",
            "unused"
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        let group = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_group.sql",
            format!("switch-{}", Uuid::new_v4()),
            "Switch",
            format!("test/switch-{}", Uuid::new_v4())
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(group),
                kind: "text_batch",
                group_path: Some("test/switch"),
                source_key: None,
                payloads: &[serde_json::json!({})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("switch-{}", Uuid::new_v4()),
            })
            .await
            .unwrap();
        let mut item = None;
        for _ in 0..20 {
            item = db
                .claim_items_fast()
                .await
                .unwrap()
                .into_iter()
                .find(|candidate| candidate.id == items[0]);
            if item.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let item = item.expect("the fixture item is claimable");
        let execution = item.execution_context().unwrap();
        let fence = TaskWriteFence::from_claimed_item(&item).unwrap();
        let store = LibraryStore::new(db.clone());

        let old_object = Uuid::new_v4();
        let old_key = format!("objects/{old_object}");
        sqlx::query_file!(
            "tests/library_ingest_switch/insert_object.sql",
            old_object,
            group,
            "0".repeat(64),
            4_i64,
            old_key
        )
        .execute(db.pool())
        .await
        .unwrap();
        let file = NewLibraryFile {
            id: Uuid::new_v4(),
            folder_id: None,
            external_id: None,
            filename: "switch.txt".into(),
            media_type: "text/plain".into(),
            size_bytes: 4,
            sha256: "0".repeat(64),
            storage_rel_path: old_key,
            storage_object_id: Some(old_object),
            delete_source_after_processing: false,
        };
        store
            .create_file_in_project_fenced(fence, group, &file)
            .await
            .unwrap()
            .unwrap();
        assert!(
            db.set_task_item_file(task_id, item.id, &execution, file.id)
                .await
                .unwrap()
        );

        SwitchFixture {
            db,
            store,
            group,
            user,
            task_id,
            item_id: item.id,
            file_id: file.id,
            old_object,
            fence,
            execution,
        }
    }

    /// The document payload of one section.
    pub fn payload(&self, external_id: &str, record_hash: &str, chunk_id: Uuid) -> ChunkPayload {
        ChunkPayload {
            chunk_id,
            document_id: 0,
            group_id: self.group,
            group_key: format!("switch-{}", self.group),
            group_path: format!("test/switch-{}", self.group),
            visibility: Visibility::Private,
            source_key: "library".into(),
            external_id: external_id.into(),
            title: "test".into(),
            summary: None,
            source_uri: "context69://test".into(),
            published_at: None,
            updated_at_source: chrono::Utc::now(),
            record_hash: record_hash.into(),
            chunk_index: 0,
            chunk_text: "content".into(),
            metadata_json: serde_json::json!({}),
            content_locale: "original".into(),
            source_locale: None,
            translation_provider: None,
        }
    }

    /// The file link of one section.
    pub fn link(&self, section_key: &str, document_id: i64) -> LibraryFileDocumentRecord {
        LibraryFileDocumentRecord {
            file_id: self.file_id,
            document_id,
            group_id: self.group,
            visibility: Visibility::Private,
            section_key: section_key.into(),
            section_label: section_key.into(),
            section_external_id: None,
            section_source_uri: None,
            section_published_at: None,
            section_metadata_json: serde_json::json!({}),
            sort_order: 0,
        }
    }

    /// A document with one chunk and one file link, as the file had it before
    /// the replacement.
    pub async fn seed_section(&self, section_key: &str, record_hash: &str) -> SeededSection {
        let external_id = format!("{section_key}-{}", Uuid::new_v4());
        let mut payload = self.payload(&external_id, record_hash, Uuid::new_v4());
        let upserted = self.db.upsert_document(&payload).await.unwrap();
        payload.document_id = upserted.document_id;
        let chunk = DocumentChunk {
            id: Uuid::new_v4(),
            document_id: upserted.document_id,
            chunk_index: 0,
            text: "old".into(),
            record_hash: record_hash.into(),
        };
        self.db
            .insert_document_chunks(upserted.document_id, record_hash, &[chunk.clone()])
            .await
            .unwrap();
        let mut connection = self.db.pool().acquire().await.unwrap();
        self.store
            .merge_file_documents_on(
                &mut connection,
                self.file_id,
                &[self.link(section_key, upserted.document_id)],
            )
            .await
            .unwrap();
        SeededSection {
            document_id: upserted.document_id,
            chunk_id: chunk.id,
            external_id,
            record_hash: record_hash.into(),
        }
    }

    /// A document with one chunk and one file link, from an explicit payload.
    pub async fn seed_document(
        &self,
        section_key: &str,
        payload: &context69::domain::ChunkPayload,
    ) -> SeededSection {
        let upserted = self.db.upsert_document(payload).await.unwrap();
        let chunk = DocumentChunk {
            id: Uuid::new_v4(),
            document_id: upserted.document_id,
            chunk_index: 0,
            text: payload.chunk_text.clone(),
            record_hash: payload.record_hash.clone(),
        };
        self.db
            .insert_document_chunks(upserted.document_id, &payload.record_hash, &[chunk.clone()])
            .await
            .unwrap();
        let mut connection = self.db.pool().acquire().await.unwrap();
        self.store
            .merge_file_documents_on(
                &mut connection,
                self.file_id,
                &[self.link(section_key, upserted.document_id)],
            )
            .await
            .unwrap();
        SeededSection {
            document_id: upserted.document_id,
            chunk_id: chunk.id,
            external_id: payload.external_id.clone(),
            record_hash: payload.record_hash.clone(),
        }
    }

    /// A document id reserved from the document sequence.
    pub async fn reserve_document_id(&self) -> i64 {
        let mut connection = self.db.pool().acquire().await.unwrap();
        self.db
            .reserve_document_id_on(&mut connection)
            .await
            .unwrap()
    }

    /// The durable run of the replacement.
    pub async fn create_run(&self) -> (Uuid, i64) {
        let mut write = begin_task_write(&self.db, self.fence).await.unwrap();
        let run = self
            .db
            .claim_or_create_library_ingest_run_in(
                &mut write,
                self.task_id,
                self.file_id,
                Some(NewIngestRun {
                    mode: IngestRunMode::Replacement,
                    source: IngestRunSource::Stored,
                }),
            )
            .await
            .unwrap();
        assert!(write.commit().await.unwrap());
        (run.batch_id, run.generation)
    }

    /// The registry entry that admits the staged point set of the run.
    pub async fn admit(
        &self,
        run_id: Uuid,
        generation: i64,
        document_ids: Vec<i64>,
        staged_ids: Vec<Uuid>,
        anticipated_retired: i64,
    ) {
        let entry = SearchRegistryEntry {
            run_id,
            file_id: Some(self.file_id),
            document_ids,
            staged_ids,
            retired_ids: Vec::new(),
            baseline_record_hash: None,
            staged_record_hash: None,
            generation,
            phase: "staging".into(),
        };
        let mut tx = self.db.pool().begin().await.unwrap();
        let admitted = self
            .db
            .admit_search_registry_entry_checked_in(
                &entry,
                Some(&context69::db::RegistryVectorIdentity {
                    embedding_fingerprint: "switch-fingerprint".into(),
                    collection_name: "switch-collection".into(),
                }),
                anticipated_retired,
                RegistryQuotaOverride::exact([1000, 1000, 1000]),
                &mut tx,
            )
            .await
            .unwrap();
        assert!(matches!(
            admitted,
            context69::db::AdmissionOutcome::Admitted(_)
        ));
        tx.commit().await.unwrap();
    }

    /// The baseline of the library file, as the replacement was prepared
    /// against it.
    pub async fn baseline(&self) -> ReplacementFileBaseline {
        let mut connection = self.db.pool().acquire().await.unwrap();
        let snapshot = self
            .store
            .lock_replacement_file_on(&mut connection, self.file_id)
            .await
            .unwrap()
            .unwrap();
        ReplacementFileBaseline {
            group_id: snapshot.group_id,
            folder_id: snapshot.folder_id,
            external_id: snapshot.external_id.clone(),
            source_uri: snapshot.source_uri.clone(),
            published_at: snapshot.published_at,
            metadata_json: snapshot.metadata_json.clone(),
            filename: snapshot.filename.clone(),
            media_type: snapshot.media_type.clone(),
            size_bytes: snapshot.size_bytes,
            sha256: snapshot.sha256.clone(),
            storage_rel_path: snapshot.storage_rel_path.clone(),
            storage_object_id: snapshot.storage_object_id,
            source_released_at: snapshot.source_released_at,
            updated_at: snapshot.updated_at,
        }
    }

    /// The staged source object of the task item.
    pub async fn attach_staged_object(&self) -> Uuid {
        let object = self.create_object().await;
        self.attach_object(object).await;
        object
    }

    /// A storage object of the group that no task item references.
    pub async fn create_object(&self) -> Uuid {
        let object = Uuid::new_v4();
        let key = format!("objects/{object}");
        sqlx::query_file!(
            "tests/library_ingest_switch/insert_object.sql",
            object,
            self.group,
            "1".repeat(64),
            8_i64,
            key
        )
        .execute(self.db.pool())
        .await
        .unwrap();
        object
    }

    /// The attachment of a storage object to the task item.
    pub async fn attach_object(&self, object: Uuid) {
        assert!(
            self.db
                .set_task_item_input_storage_object(
                    self.item_id,
                    &self.execution,
                    object,
                    &serde_json::json!({}),
                )
                .await
                .unwrap()
        );
    }

    /// The prepared replacement of one existing section.
    pub fn plan_section(
        &self,
        run_id: Uuid,
        baseline: ReplacementFileBaseline,
        section: &SeededSection,
        section_key: &str,
        record_hash: &str,
        chunk_id: Uuid,
        staged_source_object: Option<Uuid>,
    ) -> PreparedReplacement {
        let mut payload = self.payload(&section.external_id, record_hash, chunk_id);
        payload.document_id = section.document_id;
        PreparedReplacement {
            run_id,
            file_id: self.file_id,
            mode: IngestRunMode::Replacement,
            source: IngestRunSource::Stored,
            baseline,
            baseline_documents: vec![ReplacementDocumentBaseline {
                document_id: section.document_id,
                record_hash: section.record_hash.clone(),
            }],
            sections: vec![PreparedReplacementSection {
                document_id: section.document_id,
                payload,
                chunks: vec![DocumentChunk {
                    id: chunk_id,
                    document_id: section.document_id,
                    chunk_index: 0,
                    text: "new".into(),
                    record_hash: record_hash.into(),
                }],
                link: self.link(section_key, section.document_id),
            }],
            attributes: None,
            staged_source_object,
        }
    }

    /// One more section of the replacement, under an id that was reserved from
    /// the document sequence before the replacement was staged.
    pub fn add_new_section(
        &self,
        plan: &mut PreparedReplacement,
        document_id: i64,
        section_key: &str,
        external_id: &str,
        record_hash: &str,
        chunk_id: Uuid,
    ) {
        let mut payload = self.payload(external_id, record_hash, chunk_id);
        payload.document_id = document_id;
        plan.sections.push(PreparedReplacementSection {
            document_id,
            payload,
            chunks: vec![DocumentChunk {
                id: chunk_id,
                document_id,
                chunk_index: 0,
                text: "new".into(),
                record_hash: record_hash.into(),
            }],
            link: self.link(section_key, document_id),
        });
    }

    /// The library file record of the fixture, as the runner reads it.
    pub async fn file_record(&self) -> context69::domain::LibraryFileRecord {
        self.store
            .get_file(self.file_id)
            .await
            .unwrap()
            .expect("the fixture file exists")
    }

    /// The source pointer and status columns of the fixture file.
    pub async fn file_source(&self) -> FileSourceRow {
        sqlx::query_file_as!(
            FileSourceRow,
            "tests/library_ingest_switch/file_source.sql",
            self.file_id
        )
        .fetch_one(self.db.pool())
        .await
        .unwrap()
    }

    /// The stored registry entry of a run.
    pub async fn registry_entry(&self, run_id: Uuid) -> SearchRegistryEntry {
        let state = self.db.read_search_registry_state().await.unwrap();
        state
            .entries
            .into_iter()
            .find(|entry| entry.run_id == run_id)
            .expect("the registry entry of the run")
    }
}
