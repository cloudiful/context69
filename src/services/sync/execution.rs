use super::*;
use std::time::Instant;

use crate::chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, original_chunk, seed};
use crate::domain_errors::DomainError;
use crate::vector_runtime::RebuildAdmissionError;

impl SyncService {
    pub async fn sync_all(&self, trigger: &str) -> Result<()> {
        let source_keys = self.registry.read().await.source_keys();
        if source_keys.is_empty() {
            return Ok(());
        }
        let runtime = self.runtime()?;
        let results = stream::iter(source_keys.into_iter().map(|source_key| {
            let service = self.clone();
            let trigger = trigger.to_string();
            let runtime = runtime.clone();
            async move {
                let _ = runtime;
                service.sync_source(&source_key, &trigger).await
            }
        }))
        .buffer_unordered(self.max_concurrency)
        .collect::<Vec<_>>()
        .await;

        let mut errors = Vec::new();
        for result in results {
            if let Err(error) = result {
                errors.push(error.to_string());
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(DomainError::internal(errors.join("; ")).into())
        }
    }

    pub async fn sync_source(&self, source_key: &str, trigger: &str) -> Result<SyncOutcome> {
        let started = Instant::now();
        let _guard = self.acquire_lock(source_key).await?;
        let (source, connector) = self
            .source_runtime(source_key)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown source {source_key}")))
            .map_err(anyhow::Error::from)?;
        let run = self.db.start_run(source_key, trigger).await?;

        match self.sync_source_inner(&source, connector).await {
            Ok(outcome) => {
                self.db
                    .finish_run(&run, "completed", &outcome, None)
                    .await?;
                if outcome.records_changed > 0 {
                    let generation = self.db.bump_search_generation().await?;
                    info!(
                        source_key,
                        generation, "search generation bumped after source sync"
                    );
                }
                info!(
                    source_key,
                    records_seen = outcome.records_seen,
                    records_changed = outcome.records_changed,
                    chunks_upserted = outcome.chunks_upserted,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "source sync completed"
                );
                Ok(outcome)
            }
            Err(error) => {
                let empty_outcome = SyncOutcome {
                    records_seen: 0,
                    records_changed: 0,
                    chunks_upserted: 0,
                };
                self.db
                    .finish_run(&run, "failed", &empty_outcome, Some(&error.to_string()))
                    .await?;
                error!(
                    source_key,
                    error = %error,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "source sync failed"
                );
                Err(error)
            }
        }
    }

    async fn sync_source_inner(
        &self,
        source: &SourceConfig,
        connector: Arc<dyn SourceConnector>,
    ) -> Result<SyncOutcome> {
        let runtime = self.runtime()?;
        // One provider for the whole run: the snapshot pairs it with the index
        // of the same generation, so a settings save mid-sync cannot switch
        // providers between records and a rebuild is fenced out by readiness.
        let embedding = runtime.embedding.clone();
        let persisted_checkpoint = if source.sync_strategy == SyncStrategy::Cursor {
            self.db.get_checkpoint(&source.key).await?
        } else {
            SyncCheckpoint {
                updated_at: None,
                external_id: None,
            }
        };
        let mut local_checkpoint = persisted_checkpoint.clone();
        let source_scope = self
            .source_store
            .get_source_scope(&source.key)
            .await?
            .ok_or_else(|| {
                DomainError::internal(format!("missing source scope for {}", source.key))
            })?;
        let mut outcome = SyncOutcome {
            records_seen: 0,
            records_changed: 0,
            chunks_upserted: 0,
        };

        loop {
            let batch = connector.fetch_batch(&local_checkpoint).await?;
            if batch.is_empty() {
                break;
            }

            for record in batch {
                outcome.records_seen += 1;
                let normalized = normalize_record(record);

                let group = PayloadGroup {
                    group_id: source_scope.group_id,
                    group_key: &source_scope.group_key,
                    group_path: &source_scope.group_path,
                    visibility: source_scope.visibility,
                };
                let document = PayloadDocument {
                    source_key: &source.key,
                    external_id: &normalized.external_id,
                    title: &normalized.title,
                    summary: normalized.summary.as_deref(),
                    source_uri: &normalized.source_uri,
                    published_at: normalized.published_at,
                    updated_at_source: normalized.updated_at,
                    record_hash: &normalized.record_hash,
                    metadata_json: &normalized.metadata_json,
                };
                let seed_payload = seed(&group, &document, &normalized.body_text);
                let upserted = self.db.upsert_document(&seed_payload).await?;

                if upserted.changed {
                    let existing_chunk_ids = self
                        .db
                        .list_chunk_ids_for_document(upserted.document_id)
                        .await?;
                    let chunks = chunk_document(
                        upserted.document_id,
                        &source.key,
                        &normalized,
                        &self.chunking,
                    );
                    let texts = chunks
                        .iter()
                        .map(|chunk| chunk.text.clone())
                        .collect::<Vec<_>>();
                    let embeddings = embedding.embed_texts(&texts).await?;
                    let payloads = chunks
                        .iter()
                        .map(|chunk| {
                            original_chunk(
                                &group,
                                &document,
                                ChunkRef {
                                    chunk_id: chunk.id,
                                    document_id: upserted.document_id,
                                    chunk_index: chunk.chunk_index,
                                    chunk_text: &chunk.text,
                                },
                            )
                        })
                        .collect::<Vec<_>>();
                    self.db
                        .replace_document_chunks(
                            upserted.document_id,
                            &normalized.record_hash,
                            &chunks,
                        )
                        .await?;
                    runtime
                        .index
                        .replace_document_chunks(&existing_chunk_ids, &payloads, &embeddings)
                        .await?;
                    self.translation
                        .enqueue(context69_translation::EnqueueTranslation {
                            document_id: upserted.document_id,
                            directive: None,
                        })
                        .await?;
                    outcome.records_changed += 1;
                    outcome.chunks_upserted += chunks.len();
                }

                local_checkpoint.updated_at = Some(normalized.updated_at);
                local_checkpoint.external_id = Some(normalized.external_id);
            }
        }

        if source.sync_strategy == SyncStrategy::Cursor {
            self.db
                .save_checkpoint(&source.key, &local_checkpoint)
                .await?;
        }

        Ok(outcome)
    }

    /// Re-embeds every stored chunk using exactly the admitted attempt's
    /// provider/index pair.
    ///
    /// An admitted rebuild passes the one coherent pair captured when it took
    /// the fence, so no page can switch to a provider or collection published
    /// after the work started, and progress is only recorded for the generation
    /// this attempt owns.
    async fn rebuild_index_with_snapshot(&self, attempt: &RebuildAttempt) -> Result<usize> {
        let snapshot = &attempt.snapshot;
        // One provider for the whole rebuild: the snapshot is the coherent
        // provider/index pair this rebuild re-embeds with.
        let embedding = snapshot.embedding.clone();
        let total_chunks = self.db.count_chunk_payloads_for_reindex().await?;
        self.set_vector_rebuild_progress(attempt.generation, 0, total_chunks)
            .await;
        if total_chunks == 0 {
            return Ok(0);
        }

        let mut rebuilt = 0usize;
        let mut last_document_id = None;
        let mut last_chunk_index = None;

        loop {
            let batch = self
                .db
                .list_chunk_payloads_for_reindex_page(
                    last_document_id,
                    last_chunk_index,
                    Self::REINDEX_BATCH_SIZE as i64,
                )
                .await?;
            if batch.is_empty() {
                break;
            }
            let texts = batch
                .iter()
                .map(|payload| payload.chunk_text.clone())
                .collect::<Vec<_>>();
            let embeddings = embedding.embed_texts(&texts).await?;
            snapshot
                .index
                .replace_document_chunks(&[], &batch, &embeddings)
                .await?;
            rebuilt += batch.len();
            let last = batch.last().expect("non-empty reindex batch");
            last_document_id = Some(last.document_id);
            last_chunk_index = Some(last.chunk_index);
            self.set_vector_rebuild_progress(attempt.generation, rebuilt, total_chunks)
                .await;
            info!(
                batch_size = batch.len(),
                rebuilt, total_chunks, "vector rebuild batch completed"
            );
        }

        info!(
            chunks_rebuilt = rebuilt,
            "reindexed qdrant collection from app db"
        );
        Ok(rebuilt)
    }

    pub async fn vector_index_rebuild_status(&self) -> VectorIndexRebuildStatus {
        self.vector_rebuild_status.status()
    }

    pub(crate) async fn run_vector_index_rebuild(&self) -> Result<usize> {
        let attempt = self.begin_vector_index_rebuild().await?;
        let rebuilt = match self.rebuild_index_with_snapshot(&attempt).await {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                return fail_rebuild_attempt(
                    &self.vector,
                    &self.vector_rebuild_status,
                    &attempt,
                    error,
                )
                .await;
            }
        };
        // The writer seam receives the identity the attempt actually re-embedded
        // with, including the real completed count. The production closure below
        // writes exactly that value; tests capture the same value.
        let db = self.db.clone();
        settle_successful_rebuild(
            &self.vector,
            &self.vector_rebuild_status,
            &attempt,
            rebuilt,
            move |identity| {
                Box::pin(async move {
                    db.save_vector_index_state(&crate::db::VectorIndexState {
                        collection_name: &identity.collection,
                        fingerprint: &identity.fingerprint,
                        embedding_base_url: &identity.base_url,
                        embedding_model: &identity.model,
                        dimensions: identity.dimensions,
                        rebuilt_chunks: identity.rebuilt_chunks,
                    })
                    .await
                })
            },
        )
        .await
    }

    /// Admits an explicit rebuild through the runtime's one atomic admission.
    ///
    /// The runtime validates and captures the generation, the published pair,
    /// its identity, and the configured fingerprint together under the shared
    /// exclusion, then opens the fence. A retained snapshot of a superseded
    /// generation is rejected instead of being relabelled, so a destination
    /// change that advanced the generation without publishing a replacement
    /// pair can never be rebuilt as the old collection under the new identity.
    pub(crate) async fn begin_vector_index_rebuild(&self) -> Result<RebuildAttempt> {
        if !self.vector_rebuild_status.claim_start() {
            return Err(DomainError::conflict("vector index rebuild is already running").into());
        }
        let admitted = match self.vector.try_admit_rebuild() {
            Ok(admitted) => admitted,
            Err(error) => {
                self.vector_rebuild_status.release_start();
                return Err(match error {
                    RebuildAdmissionError::NotConfigured
                    | RebuildAdmissionError::MissingIdentity
                    | RebuildAdmissionError::MissingFingerprint => {
                        DomainError::unavailable(error.message()).into()
                    }
                    RebuildAdmissionError::Busy | RebuildAdmissionError::StaleSnapshot => {
                        DomainError::conflict(error.message()).into()
                    }
                });
            }
        };
        self.vector_rebuild_status.begin(admitted.generation);
        self.vector_rebuild_status.release_start();
        Ok(RebuildAttempt {
            generation: admitted.generation,
            snapshot: admitted.snapshot,
            fingerprint: admitted.configured_fingerprint,
        })
    }

    async fn set_vector_rebuild_progress(
        &self,
        generation: u64,
        processed_chunks: usize,
        total_chunks: usize,
    ) {
        self.vector_rebuild_status
            .progress(generation, processed_chunks, total_chunks);
    }
}

/// The immutable context one admitted explicit rebuild owns.
pub(crate) struct RebuildAttempt {
    pub(crate) generation: u64,
    pub(crate) snapshot: crate::vector_runtime::VectorSnapshot,
    fingerprint: String,
}

/// The identity and count an admitted rebuild writes once it succeeds.
pub(crate) struct RebuiltIdentity {
    pub(crate) collection: String,
    pub(crate) fingerprint: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) dimensions: usize,
    pub(crate) rebuilt_chunks: usize,
}

impl RebuildAttempt {
    /// The identity this attempt actually re-embedded with, carrying the real
    /// completed count.
    ///
    /// It comes from the admitted snapshot and the fingerprint captured with it,
    /// never from the runtime's current state.
    pub(crate) fn persisted_identity(&self, rebuilt_chunks: usize) -> Result<RebuiltIdentity> {
        let identity = self.snapshot.identity.as_ref().ok_or_else(|| {
            DomainError::unavailable("vector identity is unavailable after rebuild")
        })?;
        Ok(RebuiltIdentity {
            collection: self.snapshot.collection.clone(),
            fingerprint: self.fingerprint.clone(),
            base_url: identity.base_url.clone(),
            model: identity.model.clone(),
            dimensions: identity.dimensions,
            rebuilt_chunks,
        })
    }
}

/// Holds the shared exclusion across completion, so a reconfiguration cannot
/// advance the generation between the stale check, the state write, and the
/// settlement.
struct CompletionExclusionGuard(crate::vector_runtime::VectorRuntime);

impl Drop for CompletionExclusionGuard {
    fn drop(&mut self) {
        self.0.end_identity_transition();
    }
}

/// Waits for completion ownership of the shared exclusion, bounded and
/// abandoned as soon as this attempt's generation is superseded.
async fn acquire_completion_exclusion(
    vector: &crate::vector_runtime::VectorRuntime,
    generation: u64,
) -> Option<CompletionExclusionGuard> {
    for _ in 0..600 {
        if vector.try_begin_rebuild_settlement() {
            if vector.is_current_generation(generation) {
                return Some(CompletionExclusionGuard(vector.clone()));
            }
            vector.end_identity_transition();
            return None;
        }
        if !vector.is_current_generation(generation) {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    None
}

/// Settles a failed attempt: closes the fence, latches the rebuild requirement
/// for the owned generation, records `Failed` for that generation, and returns
/// the original error. A superseded generation mutates no newer state or status.
async fn fail_rebuild_attempt(
    vector: &crate::vector_runtime::VectorRuntime,
    tracker: &VectorRebuildTracker,
    attempt: &RebuildAttempt,
    error: anyhow::Error,
) -> Result<usize> {
    vector.fail_rebuild(attempt.generation);
    let message = DomainError::internal(error.to_string()).to_string();
    tracker.fail(attempt.generation, message);
    Err(error)
}

/// Settles a successful re-embedding only when this attempt still owns the
/// runtime generation.
///
/// The shared exclusion is held from before the stale check through the state
/// write and the atomic settlement, so a reconfiguration cannot advance the
/// generation after the check and leave an identity row for work that is no
/// longer current. A rejected completion never writes the identity row and
/// never clears a newer requirement or status.
async fn settle_successful_rebuild(
    vector: &crate::vector_runtime::VectorRuntime,
    tracker: &VectorRebuildTracker,
    attempt: &RebuildAttempt,
    rebuilt: usize,
    persist: impl FnOnce(
        RebuiltIdentity,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>,
) -> Result<usize> {
    let identity = attempt.persisted_identity(rebuilt)?;
    let Some(_completion) = acquire_completion_exclusion(vector, attempt.generation).await else {
        return discard_superseded_rebuild(tracker, attempt).await;
    };
    if !vector.is_current_generation(attempt.generation) {
        return discard_superseded_rebuild(tracker, attempt).await;
    }
    // Persist the identity actually used by this attempt, before the atomic
    // settlement. The row is keyed by the captured collection, so it can never
    // describe a collection configured after the work started.
    if let Err(error) = persist(identity).await {
        return fail_rebuild_attempt(vector, tracker, attempt, error).await;
    }
    if !vector.complete_rebuild_success(attempt.generation) {
        return discard_superseded_rebuild(tracker, attempt).await;
    }
    tracker.succeed(attempt.generation, rebuilt);
    Ok(rebuilt)
}

async fn discard_superseded_rebuild(
    tracker: &VectorRebuildTracker,
    attempt: &RebuildAttempt,
) -> Result<usize> {
    let error: anyhow::Error = DomainError::conflict(
        "the vector runtime changed while the rebuild ran; this rebuild result was discarded",
    )
    .into();
    let message = DomainError::internal(error.to_string()).to_string();
    tracker.fail(attempt.generation, message);
    Err(error)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        RebuildAttempt, RebuiltIdentity, VectorRebuildTracker, discard_superseded_rebuild,
        fail_rebuild_attempt, settle_successful_rebuild,
    };
    use crate::{
        contracts::{VectorIndexRebuildState, VectorIndexRebuildStatus},
        embedding::EmbeddingIdentity,
        qdrant_index::QdrantIndex,
        vector_runtime::{RebuildAdmissionError, VectorRuntime, VectorSnapshot},
    };

    fn identity(model: &str) -> EmbeddingIdentity {
        EmbeddingIdentity {
            base_url: "https://embeddings.example/v1".to_string(),
            model: model.to_string(),
            dimensions: 8,
        }
    }

    fn snapshot(generation: u64, collection: &str, model: &str) -> VectorSnapshot {
        VectorSnapshot {
            embedding: Arc::new(TestProvider),
            identity: Some(identity(model)),
            index: Arc::new(QdrantIndex::for_test(collection, 8)),
            collection: collection.to_string(),
            generation,
        }
    }

    /// A runtime with a genuinely published, fingerprinted pair at its current
    /// generation, exactly as recovery publishes one.
    fn runtime() -> VectorRuntime {
        let runtime =
            VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let generation = runtime.generation();
        assert!(runtime.publish(generation, snapshot(generation, "collection-a", "model-a")));
        runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));
        runtime
    }

    struct TestProvider;

    #[async_trait::async_trait]
    impl crate::embedding::EmbeddingProvider for TestProvider {
        async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.0; 8]).collect())
        }
    }

    /// Admits an attempt through the production admission API, not a fabricated
    /// matching snapshot.
    fn admit(runtime: &VectorRuntime) -> RebuildAttempt {
        let admitted = runtime
            .try_admit_rebuild()
            .expect("the current published generation is admitted");
        RebuildAttempt {
            generation: admitted.generation,
            snapshot: admitted.snapshot,
            fingerprint: admitted.configured_fingerprint,
        }
    }

    fn tracker_owning(generation: u64) -> VectorRebuildTracker {
        let tracker = VectorRebuildTracker::new();
        tracker.begin(generation);
        tracker
    }

    fn status_of(tracker: &VectorRebuildTracker) -> VectorIndexRebuildStatus {
        tracker.status()
    }

    #[tokio::test]
    async fn a_successful_rebuild_writes_the_actual_count_with_the_admitted_identity() {
        let runtime = runtime();
        let attempt = admit(&runtime);
        let tracker = tracker_owning(attempt.generation);
        let captured = Arc::new(std::sync::Mutex::new(Vec::<RebuiltIdentity>::new()));
        let recorded = captured.clone();

        let result = settle_successful_rebuild(&runtime, &tracker, &attempt, 12, move |identity| {
            recorded.lock().expect("recorded").push(identity);
            Box::pin(async { Ok(()) })
        })
        .await;

        assert_eq!(result.expect("the rebuild succeeds"), 12);
        let captured = captured.lock().expect("captured");
        let identity = captured.first().expect("the writer seam was called");
        assert_eq!(
            identity.rebuilt_chunks, 12,
            "the production writer receives the real completed count, never zero"
        );
        assert_eq!(identity.collection, "collection-a");
        assert_eq!(identity.fingerprint, "fingerprint-a");
        assert_eq!(identity.base_url, "https://embeddings.example/v1");
        assert_eq!(identity.model, "model-a");
        assert_eq!(identity.dimensions, 8);

        let status = status_of(&tracker);
        assert_eq!(status.state, VectorIndexRebuildState::Succeeded);
        assert_eq!(status.processed_chunks, 12);
        assert!(!runtime.is_rebuilding());
        assert!(runtime.is_ready(), "a successful rebuild reopens the pair");
        assert!(!runtime.requires_rebuild());
    }

    #[tokio::test]
    async fn an_empty_rebuild_writes_an_actual_zero_count() {
        let runtime = runtime();
        let attempt = admit(&runtime);
        let tracker = tracker_owning(attempt.generation);
        let captured = Arc::new(AtomicUsize::new(usize::MAX));
        let recorded = captured.clone();

        settle_successful_rebuild(&runtime, &tracker, &attempt, 0, move |identity| {
            recorded.store(identity.rebuilt_chunks, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        })
        .await
        .expect("the empty rebuild succeeds");

        assert_eq!(captured.load(Ordering::SeqCst), 0);
        assert_eq!(
            status_of(&tracker).state,
            VectorIndexRebuildState::Succeeded
        );
    }

    #[tokio::test]
    async fn a_state_write_failure_latches_and_allows_an_explicit_retry() {
        // Starts ready and matching: `requires_rebuild` is false before the
        // explicit rebuild, which is the reviewed failure case.
        let runtime = runtime();
        assert!(runtime.is_ready());
        assert!(!runtime.requires_rebuild());
        let attempt = admit(&runtime);
        let tracker = tracker_owning(attempt.generation);

        let outcome = settle_successful_rebuild(&runtime, &tracker, &attempt, 12, |_identity| {
            Box::pin(async { Err(anyhow::anyhow!("vector_index_state write failed")) })
        })
        .await;
        let error = outcome.expect_err("the persistence failure is returned");
        assert!(error.to_string().contains("write failed"), "{error}");
        {
            let status = status_of(&tracker);
            assert_eq!(status.state, VectorIndexRebuildState::Failed);
            assert!(status.finished_at.is_some());
            assert!(status.error_message.is_some());
        }
        assert!(!runtime.is_rebuilding(), "the fence must not stay open");
        assert!(!runtime.is_ready());
        assert!(
            runtime.requires_rebuild(),
            "a failed rebuild latches the requirement even from a ready start"
        );
        assert!(
            !runtime.restore_ready(attempt.generation),
            "a liveness probe cannot clear a failed rebuild"
        );
        runtime.commit_readiness(true);
        assert!(!runtime.is_ready());
        assert!(runtime.require_ready().is_err());

        // A later explicit attempt is admitted and can succeed.
        let retry = admit(&runtime);
        let retry_tracker = tracker_owning(retry.generation);
        let succeeded = settle_successful_rebuild(&runtime, &retry_tracker, &retry, 12, |_| {
            Box::pin(async { Ok(()) })
        })
        .await;
        assert_eq!(succeeded.expect("the retry succeeds"), 12);
        assert!(runtime.is_ready());
        assert!(!runtime.requires_rebuild());
    }

    #[tokio::test]
    async fn a_superseded_success_cannot_clear_a_newer_requirement_or_write_its_identity() {
        let runtime = runtime();
        let attempt = admit(&runtime);

        // A committed destination change while the rebuild ran publishes a new
        // pair at a newer generation and latches the new requirement.
        let attempt_generation = attempt.generation;
        let newer = runtime.begin_generation();
        assert!(newer > attempt_generation);
        assert!(runtime.publish(newer, snapshot(newer, "collection-b", "model-b")));
        runtime.set_configured_fingerprint(Some("fingerprint-b".to_string()));
        runtime.set_requires_rebuild(true);

        // A newer attempt owns the status while the older one settles.
        let tracker = tracker_owning(newer);
        let persisted = Arc::new(AtomicUsize::new(0));
        let recorded = persisted.clone();
        let outcome = settle_successful_rebuild(&runtime, &tracker, &attempt, 12, move |_| {
            recorded.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        })
        .await;

        let error = outcome.expect_err("a superseded completion is discarded");
        assert!(error.to_string().contains("discarded"), "{error}");
        assert_eq!(
            persisted.load(Ordering::SeqCst),
            0,
            "a superseded attempt must not write its identity row"
        );
        let status = status_of(&tracker);
        assert_eq!(
            status.state,
            VectorIndexRebuildState::Running,
            "a stale completion must not replace the newer Running status"
        );
        assert_eq!(
            runtime.generation(),
            newer,
            "the newer generation is untouched"
        );
        assert!(
            runtime.requires_rebuild(),
            "the newer requirement must stay latched"
        );
        assert!(!runtime.is_ready());
        assert!(
            !runtime.restore_ready(newer),
            "a probe still cannot reopen a newer required space"
        );
    }

    #[tokio::test]
    async fn a_stale_failure_does_not_latch_or_disturb_a_newer_generation_or_status() {
        let runtime = runtime();
        let attempt = admit(&runtime);
        let newer = runtime.begin_generation();
        let tracker = tracker_owning(newer);

        let outcome = fail_rebuild_attempt(
            &runtime,
            &tracker,
            &attempt,
            anyhow::anyhow!("stale embedding failure"),
        )
        .await;
        assert!(outcome.is_err());
        assert!(
            !runtime.requires_rebuild(),
            "a stale failure must not alter the newer generation"
        );
        assert!(!runtime.is_rebuilding());
        assert_eq!(runtime.generation(), newer);
        assert_eq!(
            status_of(&tracker).state,
            VectorIndexRebuildState::Running,
            "a stale failure must not finish a newer attempt"
        );
    }

    #[test]
    fn only_the_owning_generation_can_mutate_progress_or_finish_status() {
        let tracker = tracker_owning(7);

        assert!(!tracker.progress(6, 5, 10));
        assert!(!tracker.succeed(6, 10));
        assert!(!tracker.fail(6, "stale".to_string()));
        let status = status_of(&tracker);
        assert_eq!(status.state, VectorIndexRebuildState::Running);
        assert_eq!(status.processed_chunks, 0);

        assert!(tracker.progress(7, 5, 10));
        assert!(tracker.succeed(7, 10));
        let status = status_of(&tracker);
        assert_eq!(status.state, VectorIndexRebuildState::Succeeded);
        assert_eq!(status.processed_chunks, 10);
        assert_eq!(status.total_chunks, 10);

        // A later generation cannot resurrect the finished attempt.
        assert!(!tracker.progress(8, 1, 1));
        assert!(!tracker.fail(8, "newer".to_string()));
        assert_eq!(
            status_of(&tracker).state,
            VectorIndexRebuildState::Succeeded
        );
    }

    #[test]
    fn production_admission_rejects_a_retained_snapshot_after_a_generation_bump() {
        let runtime = runtime();
        let generation = runtime.generation();
        let retained = runtime.snapshot().expect("published pair");

        // A destination reconfiguration advances the generation and retains the
        // old snapshot for diagnostics, without publishing a replacement pair.
        assert!(runtime.try_begin_identity_transition());
        let (changed, new_generation) = runtime
            .apply_runtime_reconfiguration(Some("fingerprint-b".to_string()), || true)
            .expect("no fence is open");
        runtime.end_identity_transition();
        assert!(changed);
        assert!(new_generation > generation);

        let error = runtime
            .try_admit_rebuild()
            .err()
            .expect("a retained snapshot of a superseded configuration is rejected");
        assert_eq!(error, RebuildAdmissionError::StaleSnapshot);
        assert!(
            !runtime.is_rebuilding(),
            "the rejected admission opens no fence"
        );
        assert_eq!(
            runtime.snapshot().map(|snapshot| snapshot.collection),
            Some(retained.collection),
            "the rejected admission relabels nothing"
        );
        let first_generation = runtime.generation();
        assert_eq!(first_generation, new_generation);

        // Once the new destination publishes its own pair, admission captures
        // that pair and its fingerprint together.
        assert!(runtime.publish(
            new_generation,
            snapshot(new_generation, "collection-b", "model-b")
        ));
        let admitted = runtime
            .try_admit_rebuild()
            .expect("the current replacement pair is rebuildable");
        assert_eq!(admitted.snapshot.collection, "collection-b");
        assert_eq!(admitted.snapshot.generation, admitted.generation);
        assert_eq!(admitted.configured_fingerprint, "fingerprint-b");
        assert!(runtime.is_rebuilding());
    }

    #[tokio::test]
    async fn a_discarded_completion_never_claims_success_for_its_generation() {
        let runtime = runtime();
        let attempt = admit(&runtime);
        let tracker = tracker_owning(attempt.generation);
        let outcome = discard_superseded_rebuild(&tracker, &attempt).await;
        assert!(outcome.is_err());
        let status = status_of(&tracker);
        assert_eq!(status.state, VectorIndexRebuildState::Failed);
        assert!(status.finished_at.is_some());
    }
}
