//! Single-batch persistence for the checkpointed indexing stage.
//!
//! Persists one batch in the order the checkpoint protocol requires: SQL
//! chunk insert, then the Qdrant upsert, and only then the lease-conditional
//! checkpoint write; the in-memory checkpoint and payload advance only after
//! all three succeed.

use anyhow::anyhow;
use serde_json::Value;

use super::ingest_checkpoint::{
    INDEXING_CHECKPOINT_VERSION, IndexingCheckpoint, payload_with_checkpoint,
};
use super::ingest_progress::{PipelineProgress, PipelineProgressStage};
use super::ingest_progress_codec::payload_with_progress;
use super::ingest_types::{IngestFailure, PreparedIngestSection};
use super::task_ingest::{normalize_task_failure, task_failure};
use super::{LibraryRuntime, LibraryService, UnifiedIngestError};
use crate::chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, original_chunk};
use crate::contracts::LibraryIngestFailureStage;

/// Immutable inputs for one indexing batch. Grouped so the batch-persist
/// helper stays within Clippy's argument-count budget without a lint
/// suppression.
pub(super) struct BatchPersistInputs<'a> {
    pub(super) file: &'a crate::domain::LibraryFileRecord,
    pub(super) prepared_section: &'a PreparedIngestSection,
    pub(super) document_id: i64,
    pub(super) batch: &'a [crate::domain::DocumentChunk],
    pub(super) this_batch_index: usize,
    pub(super) total_batches: usize,
    /// Chunks the whole run will produce, counted by the same pass that
    /// produced `total_batches`.
    pub(super) total_chunks: usize,
    /// Chunks covered by the batches before this one, including the batches a
    /// resume skipped.
    pub(super) completed_chunks_before: usize,
    pub(super) current_hash: &'a str,
    pub(super) attempt: i32,
    /// Whoever authorizes this run's payload checkpoint.
    pub(super) checkpoint_write: super::ingest_checkpoint_persistence::CheckpointWrite,
}

impl LibraryService {
    /// The content of one batch: the embedding, the chunk write and the vector
    /// upsert.
    ///
    /// The embedding runs before any transaction is open, the chunks are
    /// written inside the guarded transaction of the captured grant, and the
    /// vector upsert runs outside every transaction after the grant was checked
    /// again.
    pub(super) async fn persist_batch_content(
        &self,
        fence: Option<crate::library_store::TaskWriteFence>,
        file: &crate::domain::LibraryFileRecord,
        prepared_section: &PreparedIngestSection,
        document_id: i64,
        batch: &[crate::domain::DocumentChunk],
        runtime: &LibraryRuntime,
    ) -> Result<(), UnifiedIngestError> {
        let texts = batch
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        // One provider for the batch: a concurrent settings save must not switch
        // providers within a single Qdrant upsert. `require_ready` also makes the
        // writer wait out an identity-changing rebuild, so it cannot add vectors
        // with one identity to a collection being re-embedded with another.
        let embedding = runtime.embedding.require_ready().map_err(|error| {
            normalize_task_failure(IngestFailure::new(
                LibraryIngestFailureStage::Embedding,
                error,
            ))
        })?;
        let embeddings = embedding.embed_texts(&texts).await.map_err(|error| {
            let failure = IngestFailure::new(LibraryIngestFailureStage::Embedding, error);
            normalize_task_failure(failure)
        })?;
        let group = PayloadGroup {
            group_id: file.group_id,
            group_key: &file.group_key,
            group_path: &file.group_path,
            visibility: file.visibility,
        };
        let document = PayloadDocument {
            source_key: super::FILE_LIBRARY_SOURCE_KEY,
            external_id: &prepared_section.normalized.external_id,
            title: &prepared_section.normalized.title,
            summary: prepared_section.normalized.summary.as_deref(),
            source_uri: &prepared_section.normalized.source_uri,
            published_at: prepared_section.normalized.published_at,
            updated_at_source: prepared_section.normalized.updated_at,
            record_hash: &prepared_section.normalized.record_hash,
            metadata_json: &prepared_section.normalized.metadata_json,
        };
        let payloads = batch
            .iter()
            .map(|chunk| {
                original_chunk(
                    &group,
                    &document,
                    ChunkRef {
                        chunk_id: chunk.id,
                        document_id,
                        chunk_index: chunk.chunk_index,
                        chunk_text: &chunk.text,
                    },
                )
            })
            .collect::<Vec<_>>();

        // SQL persist; idempotent under deterministic chunk IDs.
        match fence {
            Some(fence) => {
                // The chunks are written inside the guarded transaction of the
                // captured grant, so the chunks and the authority that produced
                // them commit together.
                let mut write = crate::library_store::begin_task_write(&self.db, fence)
                    .await
                    .map_err(|error| {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        normalize_task_failure(failure)
                    })?;
                sqlx::query_file!("src/sql/db/documents/savepoint_chunk_insert.sql")
                    .execute(write.connection())
                    .await
                    .map_err(|error| {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        normalize_task_failure(failure)
                    })?;
                let inserted = self
                    .db
                    .insert_document_chunks_in(
                        write.connection(),
                        document_id,
                        &prepared_section.normalized.record_hash,
                        batch,
                    )
                    .await;
                match inserted {
                    Ok(()) => {
                        sqlx::query_file!("src/sql/db/documents/release_chunk_insert.sql")
                            .execute(write.connection())
                            .await
                            .map_err(|error| {
                                let failure =
                                    IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                                normalize_task_failure(failure)
                            })?;
                    }
                    Err(error) => {
                        let msg = error.to_string().to_ascii_lowercase();
                        if msg.contains("duplicate key") || msg.contains("unique constraint") {
                            // Document chunks primary key collision => already
                            // inserted by an earlier partial ingest. The
                            // statement is rolled back to its savepoint and the
                            // batch counts as persisted.
                            sqlx::query_file!("src/sql/db/documents/rollback_to_chunk_insert.sql")
                                .execute(write.connection())
                                .await
                                .map_err(|error| {
                                    let failure = IngestFailure::new(
                                        LibraryIngestFailureStage::Indexing,
                                        error,
                                    );
                                    normalize_task_failure(failure)
                                })?;
                        } else {
                            let failure =
                                IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                            return Err(normalize_task_failure(failure));
                        }
                    }
                }
                if !write.commit().await.map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    normalize_task_failure(failure)
                })? {
                    return Err(task_failure(
                        "indexing",
                        anyhow!("task item lease was lost while writing the batch",),
                        true,
                    ));
                }
            }
            None => match self
                .db
                .insert_document_chunks(
                    document_id,
                    &prepared_section.normalized.record_hash,
                    batch,
                )
                .await
            {
                Ok(()) => {}
                Err(error) => {
                    let msg = error.to_string().to_ascii_lowercase();
                    if msg.contains("duplicate key") || msg.contains("unique constraint") {
                        // Document chunks primary key collision => already inserted
                        // by an earlier partial ingest. Treat as success.
                    } else {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        return Err(normalize_task_failure(failure));
                    }
                }
            },
        }

        // The vector upsert is an external mutation, so the grant is checked
        // again right before it: a segment that lost its claim must not leave
        // points behind that no checkpoint will ever reconcile.
        if let Some(fence) = fence {
            let granted = crate::library_store::task_write_granted(&self.db, fence)
                .await
                .map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    normalize_task_failure(failure)
                })?;
            if !granted {
                return Err(task_failure(
                    "indexing",
                    anyhow!(
                        "task item lease was lost before the vectors of the batch were written"
                    ),
                    true,
                ));
            }
        }

        if let Err(error) = runtime
            .index
            .upsert_document_chunks(&payloads, &embeddings)
            .await
        {
            let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
            return Err(normalize_task_failure(failure));
        }
        Ok(())
    }

    pub(super) async fn persist_one_batch(
        &self,
        inputs: BatchPersistInputs<'_>,
        checkpoint: &mut IndexingCheckpoint,
        current_payload_value: &mut Value,
        runtime: &LibraryRuntime,
    ) -> Result<(), UnifiedIngestError> {
        let BatchPersistInputs {
            file,
            prepared_section,
            document_id,
            batch,
            this_batch_index,
            total_batches,
            total_chunks,
            completed_chunks_before,
            current_hash,
            attempt,
            checkpoint_write,
        } = inputs;
        self.persist_batch_content(
            checkpoint_write.fence(),
            file,
            prepared_section,
            document_id,
            batch,
            runtime,
        )
        .await?;

        // Advance the checkpoint only after both stores succeeded, and publish the
        // counters in the same lease-fenced write: a chunk counts as completed
        // only once it has been embedded, persisted and upserted, so a failed
        // batch leaves the published count exactly where it was.
        let next_checkpoint = IndexingCheckpoint {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: this_batch_index + 1,
            total_batches: Some(total_batches),
            record_hash: Some(current_hash.to_string()),
        };
        let progress = PipelineProgress::new(attempt, PipelineProgressStage::Indexing)
            .with_batch_counters(
                completed_chunks_before + batch.len(),
                total_chunks,
                this_batch_index + 1,
                total_batches,
            )?
            .observed_now();
        let with_progress = payload_with_progress(current_payload_value, &progress)
            .map_err(|error| task_failure("indexing", error, false))?;
        let next_payload = payload_with_checkpoint(&with_progress, &next_checkpoint)
            .map_err(|error| task_failure("indexing", error, false))?;
        let ok = checkpoint_write
            .write(&self.db, &next_payload)
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        if !ok {
            // Lease lost or status not running; Qdrant upsert already
            // succeeded, but checkpoint stays behind. Retry re-upserts the
            // same points idempotently via deterministic chunk IDs.
            return Err(task_failure(
                "indexing",
                anyhow!(
                    "task item lease was lost while checkpointing batch {}",
                    this_batch_index
                ),
                true,
            ));
        }
        *current_payload_value = next_payload;
        *checkpoint = next_checkpoint;
        Ok(())
    }
}

#[cfg(test)]
mod provider_barrier {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::json;
    use uuid::Uuid;

    use crate::chunking::ChunkingConfig;
    use crate::config::FileLibraryConfig;
    use crate::db::Database;
    use crate::domain::{DocumentChunk, LibraryFileRecord, NormalizedDocument};
    use crate::embedding::EmbeddingProvider;
    use crate::embedding::{EmbeddingIdentity, EmbeddingRuntime};
    use crate::library_store::TaskWriteFence;
    use crate::qdrant_index::QdrantIndex;
    use crate::services::library::{LibraryRuntime, LibraryService, LibraryServiceConfig};
    use crate::services::settings::SettingsService;
    use context69_translation::{
        TranslationChunkPublication, TranslationDependencies, TranslationPublication,
        TranslationPublisher, TranslationReadiness, TranslationService,
    };

    use super::super::ingest_types::{IngestSection, PreparedIngestSection};

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
        async fn delete(&self, _chunk_ids: &[Uuid]) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl TranslationReadiness for NoopCallbacks {
        async fn is_ready(&self) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    /// The embedding provider of the barrier case.
    ///
    /// While the provider runs, the case locks the task item row. The guarded
    /// write of the ingest holds exactly that lock, so the lock proves that no
    /// transaction is held over the provider call.
    struct BarrierEmbedder {
        calls: AtomicUsize,
        db: Database,
        item_id: Uuid,
    }

    #[async_trait]
    impl EmbeddingProvider for BarrierEmbedder {
        async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut connection = self.db.pool().acquire().await?;
            let locked = sqlx::query_file_scalar!(
                "tests/sql/task_execution/lock_item_nowait.sql",
                self.item_id
            )
            .fetch_optional(&mut *connection)
            .await;
            assert!(
                locked.is_ok(),
                "no transaction may be held while the provider runs"
            );
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }
    }

    async fn connect() -> Database {
        let url = context69_test_support::accessor::database_url();
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap the library schema");
        db
    }

    /// A claimed item and its captured grant.
    async fn claim(db: &Database) -> (Uuid, TaskWriteFence) {
        let user_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_user.sql",
            format!("p3c-barrier-{}", Uuid::new_v4()),
            "P3C Barrier",
            "unused"
        )
        .fetch_one(db.pool())
        .await
        .expect("seed the user");
        let group_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_group.sql",
            format!("p3c-barrier-{}", Uuid::new_v4()),
            "P3C Barrier",
            format!("test/p3c-barrier-{}", Uuid::new_v4())
        )
        .fetch_one(db.pool())
        .await
        .expect("seed the group");
        sqlx::query_file!(
            "tests/sql/task_execution/seed_group_membership.sql",
            group_id,
            user_id
        )
        .execute(db.pool())
        .await
        .expect("seed the membership");
        let task_id = Uuid::new_v4();
        let (_, _, _items) = db
            .create_task_submission_with_input_objects(crate::db::CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: Some(group_id),
                kind: "text_batch",
                group_path: Some("test/p3c-barrier"),
                source_key: None,
                payloads: &[json!({"external_id": "a"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("p3c-barrier-{}", Uuid::new_v4()),
            })
            .await
            .expect("create the task");
        let item = db
            .claim_items_fast(50)
            .await
            .expect("claim the item")
            .into_iter()
            .find(|item| item.task_id == task_id)
            .expect("the item is claimed");
        let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
        (item.id, fence)
    }

    fn library_file(group_id: i64) -> LibraryFileRecord {
        LibraryFileRecord {
            group_id,
            group_key: "p3c-barrier".to_string(),
            group_path: "/test/p3c-barrier".to_string(),
            visibility: crate::contracts::Visibility::Public,
            id: Uuid::new_v4(),
            folder_id: None,
            external_id: Some("p3c-barrier".to_string()),
            source_uri: None,
            published_at: None,
            metadata_json: json!({}),
            filename: "p3c-barrier.txt".to_string(),
            media_type: "text/plain".to_string(),
            size_bytes: 16,
            sha256: "b".repeat(64),
            storage_rel_path: format!("objects/{}/p3c-barrier", group_id),
            ingest_status: crate::contracts::LibraryIngestStatus::Failed,
            error_message: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ingested_at: None,
        }
    }

    fn prepared_section() -> PreparedIngestSection {
        PreparedIngestSection {
            index: 0,
            section: IngestSection {
                section_key: "document".to_string(),
                section_label: "Document".to_string(),
                title: "p3c barrier".to_string(),
                summary: None,
                body_text: "p3c barrier body".to_string(),
                source_uri: None,
                external_id: None,
                published_at: None,
                metadata_json: json!({}),
            },
            normalized: NormalizedDocument {
                external_id: "p3c-barrier".to_string(),
                title: "p3c barrier".to_string(),
                summary: None,
                body_text: "p3c barrier body".to_string(),
                source_uri: "context69://library/files/p3c-barrier".to_string(),
                published_at: None,
                updated_at: chrono::Utc::now(),
                metadata_json: json!({}),
                record_hash: "p3c-barrier-hash".to_string(),
            },
        }
    }

    /// The provider runs outside every transaction of the ingest.
    ///
    /// The batch content helper embeds first, writes the chunks inside the
    /// guarded transaction of the grant and upserts the vectors afterwards. The
    /// provider locks the task item row while it runs: the guarded write holds
    /// exactly that lock, so the lock succeeding proves that no transaction is
    /// held over the provider call.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_provider_runs_outside_every_transaction() {
        let db = connect().await;
        let (item_id, fence) = claim(&db).await;

        let embedder = Arc::new(BarrierEmbedder {
            calls: AtomicUsize::new(0),
            db: db.clone(),
            item_id,
        });
        let settings = SettingsService::new(db.clone());
        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(NoopCallbacks),
            concurrency: 1,
            readiness: Arc::new(NoopCallbacks),
        });
        let qdrant = QdrantIndex::for_test("test-noop", 4);
        let service = LibraryService::new(
            db.clone(),
            Some(embedder.clone() as Arc<dyn EmbeddingProvider>),
            Some(qdrant.clone()),
            LibraryServiceConfig {
                chunking: ChunkingConfig {
                    max_chars: 200,
                    overlap_chars: 0,
                },
                file_library: FileLibraryConfig {
                    storage_root: std::env::temp_dir()
                        .join(format!("context69-p3c-barrier-{}", Uuid::new_v4())),
                    max_upload_size_mb: 1,
                    max_upload_request_size_mb: 1,
                    ingest_concurrency: 1,
                    url_import_concurrency: 1,
                    url_import_min_interval_ms: 1000,
                    trusted_proxy_enabled: false,
                    s3: None,
                },
                valkey_url: None,
                embedding_vector_configured: true,
                embedding_vector_configuration_fingerprint: "p3c-barrier".to_string(),
            },
            settings,
            translation,
        )
        .await
        .expect("build the library service");

        let runtime = LibraryRuntime {
            embedding: EmbeddingRuntime::new(
                Some(embedder.clone() as Arc<dyn EmbeddingProvider>),
                Some(EmbeddingIdentity {
                    base_url: "http://127.0.0.1:1".to_string(),
                    model: "p3c-barrier".to_string(),
                    dimensions: 4,
                }),
            ),
            index: qdrant,
        };
        let file = library_file(1);
        let section = prepared_section();
        // The chunks reference a document, so the case seeds the document row
        // the chunks belong to.
        let document_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_barrier_document.sql",
            file.group_id
        )
        .fetch_one(db.pool())
        .await
        .expect("seed the document");
        let chunk = DocumentChunk {
            id: Uuid::new_v4(),
            document_id,
            chunk_index: 0,
            text: "p3c barrier body".to_string(),
            record_hash: "p3c-barrier-hash".to_string(),
        };

        service
            .persist_batch_content(
                Some(fence),
                &file,
                &section,
                document_id,
                &[chunk],
                &runtime,
            )
            .await
            .expect("the batch content is written");

        assert!(
            embedder.calls.load(Ordering::SeqCst) > 0,
            "the provider ran"
        );
        let chunks: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_document_chunks.sql",
            document_id
        )
        .fetch_one(db.pool())
        .await
        .expect("count the chunks");
        assert_eq!(chunks, 1, "the chunks are written under the grant");

        sqlx::query_file!(
            "tests/sql/task_execution/delete_document_chunks_by_document.sql",
            document_id
        )
        .execute(db.pool())
        .await
        .expect("delete the chunks");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_document_by_id.sql",
            document_id
        )
        .execute(db.pool())
        .await
        .expect("delete the document");
    }
}
