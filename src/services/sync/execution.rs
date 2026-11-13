use super::*;
use std::time::Instant;

use context69_translation::TranslationChunkingBudget;

use crate::chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, original_chunk, seed};
use crate::chunking::{
    ResolvedChunkBudget, chunk_document_iter_resolved, parse_chunk_budget_value,
};
use crate::config::QdrantConfig;
use crate::db::{
    RebuildDocumentRow, RegistryVectorIdentity, staged_original_payload, staged_translation_payload,
};
use crate::domain::{DocumentChunk, NormalizedDocument};
use crate::domain_errors::DomainError;
use crate::embedding::embedding_runtime::vector_identity_fingerprint;
use crate::embedding::{EmbeddingIdentity, EmbeddingProvider, build_provider};
use crate::library_store::TaskWriteFence;
use crate::vector_runtime::RebuildAdmissionError;
use uuid::Uuid;

/// Source rows installed per bounded original-installation batch.
const REBUILD_INSTALL_BATCH_SIZE: i64 = 2000;

/// Old original registry entries removed per bounded cleanup batch.
const REBUILD_ORIGINAL_CLEANUP_BATCH_SIZE: i64 = 2000;

/// The deterministic run id of the original reservation of one target file.
///
/// Every installation page of the same build increments this one row, and a
/// repeated installation of the same target derives the same id instead of
/// creating another reservation.
fn original_reservation_run_id(collection: &str, file_id: Option<Uuid>) -> Uuid {
    let file = file_id.map(|id| id.to_string()).unwrap_or_default();
    let name = format!("context69-rebuild-original:{collection}:{file}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

/// Whether an exhausted staged source completes the bounded installation.
///
/// Completion requires both the exhausted source and an installed count equal
/// to the pinned total, so a zero-total build settles explicitly and a short
/// source aborts instead of publishing.
fn install_completion(installed: i64, total: i64, source_exhausted: bool) -> Result<bool> {
    if !source_exhausted {
        return Ok(false);
    }
    if installed != total {
        anyhow::bail!("the staged original source ended after {installed} of {total} chunks");
    }
    Ok(true)
}

/// Holds the single rebuild start slot until an attempt actually begins.
///
/// The reservation is armed while the attempt resolves, proves the provider
/// and prepares its target, so every return, error or cancellation releases
/// the tracker gate through `Drop`. It is disarmed only once the tracker has
/// taken ownership of the admitted generation.
struct RebuildStartReservation<'a> {
    tracker: &'a VectorRebuildTracker,
    armed: bool,
}

impl RebuildStartReservation<'_> {
    /// Releases the short-lived gate once the running tracker owns exclusion.
    fn settle(mut self) {
        self.tracker.release_start();
        self.armed = false;
    }
}

impl Drop for RebuildStartReservation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.tracker.release_start();
        }
    }
}

/// The typed marker of a rebuild refused because another live owner runs.
///
/// It is deliberately distinct from the retryable refusals (unresolved staging
/// entries) and from configuration refusals: losing the ownership race is a
/// one-shot conflict, not a provider outage to retry.
#[derive(Debug)]
pub(crate) struct RebuildBusy;

impl std::fmt::Display for RebuildBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("another vector index rebuild is already running")
    }
}

impl std::error::Error for RebuildBusy {}

/// The checked grant row of a rebuild completion.
#[derive(sqlx::FromRow)]
struct LockedCompletionItem {
    status: String,
    owned: bool,
    matches_execution: bool,
}

impl SyncService {
    pub async fn sync_all(&self, trigger: &str) -> Result<()> {
        // One budget for the whole fan-out: every source of this operation
        // chunks under the value captured here, before the first await, so a
        // settings save cannot assign different budgets to different sources
        // of one sync_all.
        let chunking = self.chunking.snapshot();
        let source_keys = self.registry.read().await.source_keys();
        if source_keys.is_empty() {
            return Ok(());
        }
        let runtime = self.runtime()?;
        let results = stream::iter(source_keys.into_iter().map(|source_key| {
            let service = self.clone();
            let trigger = trigger.to_string();
            let runtime = runtime.clone();
            let chunking = chunking.clone();
            async move {
                let _ = runtime;
                service
                    .sync_source_with_budget(&source_key, &trigger, &chunking)
                    .await
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
        let chunking = self.chunking.snapshot();
        self.sync_source_with_budget(source_key, trigger, &chunking)
            .await
    }

    /// Syncs one source under an explicit chunk budget.
    ///
    /// A task item passes the budget it pinned at submission, so a settings
    /// save cannot rechunk a multi-record sync that started under an older
    /// budget.
    pub async fn sync_source_with_budget(
        &self,
        source_key: &str,
        trigger: &str,
        chunking: &ChunkBudget,
    ) -> Result<SyncOutcome> {
        let started = Instant::now();
        let _guard = self.acquire_lock(source_key).await?;
        let (source, connector) = self
            .source_runtime(source_key)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown source {source_key}")))
            .map_err(anyhow::Error::from)?;
        let run = self.db.start_run(source_key, trigger).await?;

        match self.sync_source_inner(&source, connector, chunking).await {
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
        chunking: &ChunkBudget,
    ) -> Result<SyncOutcome> {
        let runtime = self.runtime()?;
        // The identity belongs to the very provider/index pair captured above,
        // so a settings change on another node can never be overtaken by this
        // run's writes.
        let identity =
            runtime
                .configured_fingerprint
                .as_ref()
                .map(|fingerprint| RegistryVectorIdentity {
                    embedding_fingerprint: fingerprint.clone(),
                    collection_name: runtime.collection.clone(),
                });
        let resolved_chunking = self.chunking.resolve(chunking)?;
        // The pinned budget of this source sync governs the translations it
        // induces too: the scoped service carries it as the explicit snapshot,
        // so the shared service's live default is never consulted here.
        let translation = self
            .translation
            .clone()
            .with_default_chunking(Some(TranslationChunkingBudget::from(chunking)));
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
                // The document, version and metadata rows commit with the
                // barrier and identity they were captured under. No embedding
                // work happens while this transaction is open.
                let mut tx = self.db.pool().begin().await?;
                self.db
                    .require_idle_admissible_vector_identity_in(&mut tx, identity.as_ref())
                    .await?;
                let upserted = self.db.upsert_document_in(&mut tx, &seed_payload).await?;
                tx.commit().await?;

                if upserted.changed {
                    let existing_chunk_ids = self
                        .db
                        .list_chunk_ids_for_document(upserted.document_id)
                        .await?;
                    let chunks = crate::chunking::chunk_document_resolved(
                        upserted.document_id,
                        &source.key,
                        &normalized,
                        &resolved_chunking,
                    )?;
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
                    // The chunk rows commit under the same captured identity,
                    // in their own short transaction after embedding. A rebuild
                    // that started meanwhile refuses this write, so the run
                    // cannot add rows to a collection being re-embedded.
                    let mut tx = self.db.pool().begin().await?;
                    self.db
                        .require_idle_admissible_vector_identity_in(&mut tx, identity.as_ref())
                        .await?;
                    self.db
                        .delete_document_chunks_in(&mut tx, upserted.document_id)
                        .await?;
                    self.db
                        .insert_document_chunks_in(
                            &mut tx,
                            upserted.document_id,
                            &normalized.record_hash,
                            &chunks,
                        )
                        .await?;
                    tx.commit().await?;
                    runtime
                        .index
                        .replace_document_chunks(&existing_chunk_ids, &payloads, &embeddings)
                        .await?;
                    translation
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

    /// Stages and re-embeds one document at a time using exactly the admitted
    /// attempt's pinned budget, chunk-id seed, provider and target.
    ///
    /// Staging commits one document at a time (including documents that produce
    /// no chunks), and a staged document is drained before the next one is
    /// staged, so a resume continues from the durable cursors instead of
    /// restarting the library. Progress is recorded only for the generation this
    /// attempt owns.
    async fn rebuild_index_with_snapshot(
        &self,
        attempt: &RebuildAttempt,
        fence: TaskWriteFence,
    ) -> Result<RebuildProgress> {
        // One target for the whole rebuild: the snapshot is the coherent
        // provider/index pair this rebuild re-embeds with.
        let target = attempt.snapshot.collection.as_str();

        // A document without its current canonical body cannot be re-chunked;
        // report the actionable ids once per attempt instead of skipping it.
        let missing = self
            .db
            .list_rebuild_missing_documents_page(None, Self::REINDEX_BATCH_SIZE as i64)
            .await?;
        if !missing.is_empty() {
            anyhow::bail!(
                "documents without a canonical document_versions body cannot be rebuilt: {missing:?}"
            );
        }

        let barrier = self.db.read_rebuild_barrier().await?;
        if !rebuild_is_owned(attempt, &fence, &barrier) {
            return Ok(RebuildProgress::Cancelled);
        }
        if barrier.pause_requested {
            return Ok(RebuildProgress::Paused);
        }
        let staging = self.db.read_rebuild_staging_state().await?;
        let mut stream = RebuildStream {
            staged_document_id: staging.staged_document_id,
            original_cursor: barrier.cursor,
            translation_cursor: staging.translation_cursor,
            originals: usize::try_from(barrier.processed_chunks)
                .context("the committed original count must be non-negative")?,
            translations: usize::try_from(staging.translation_processed_chunks)
                .context("the committed translation count must be non-negative")?,
            known_total: None,
        };
        if staging.staging_complete {
            stream.known_total = Some(self.count_staged_chunks(target).await?);
        }
        self.report_rebuild_progress(attempt.generation, &stream)
            .await;

        loop {
            // The durable barrier is the authority at every control boundary: a
            // pause request parks after the page this attempt already committed,
            // and a cancelled or superseded owner stops before the next write.
            let barrier = self.db.read_rebuild_barrier().await?;
            if !rebuild_is_owned(attempt, &fence, &barrier) {
                return Ok(RebuildProgress::Cancelled);
            }
            if barrier.pause_requested {
                return Ok(RebuildProgress::Paused);
            }
            let next_original = self
                .db
                .list_rebuild_staging_chunks_page(
                    target,
                    stream.original_cursor.map(|cursor| cursor.document_id),
                    stream.original_cursor.map(|cursor| cursor.chunk_index),
                    1,
                )
                .await?
                .first()
                .map(|row| row.document_id);
            let next_translation = self
                .db
                .list_translation_rebuild_staging_page(
                    target,
                    stream.translation_cursor.map(|cursor| cursor.document_id),
                    stream
                        .translation_cursor
                        .map(|cursor| cursor.translation_id),
                    stream.translation_cursor.map(|cursor| cursor.chunk_index),
                    1,
                )
                .await?
                .first()
                .map(|row| row.document_id);
            let next_document = match (next_original, next_translation) {
                (Some(original), Some(translated)) => Some(original.min(translated)),
                (original, translated) => original.or(translated),
            };
            let Some(document_id) = next_document else {
                if stream.known_total.is_some() {
                    break;
                }
                let documents = self
                    .db
                    .list_rebuild_documents_page(stream.staged_document_id, 1)
                    .await?;
                let Some(document) = documents.first() else {
                    // Every canonical document is staged: the exact total is now
                    // known and the remaining pages drain against it.
                    let total = self.count_staged_chunks(target).await?;
                    let mut connection = self.db.pool().acquire().await?;
                    let completed = self
                        .db
                        .complete_rebuild_staging_in(
                            &mut connection,
                            fence.item_id(),
                            fence.token(),
                            &attempt.fingerprint,
                            target,
                            i64::try_from(total).context("the staged total must fit in BIGINT")?,
                        )
                        .await?;
                    if !completed {
                        return Ok(RebuildProgress::Cancelled);
                    }
                    stream.known_total = Some(total);
                    self.report_rebuild_progress(attempt.generation, &stream)
                        .await;
                    continue;
                };
                let chunks = self.chunk_rebuild_document(attempt, document).await?;
                // The CPU closure only computes. Ownership and pause are
                // rechecked here, and the staging transaction below is the only
                // writer, so a dropped closure never commits anything.
                let barrier = self.db.read_rebuild_barrier().await?;
                if !rebuild_is_owned(attempt, &fence, &barrier) {
                    return Ok(RebuildProgress::Cancelled);
                }
                if barrier.pause_requested {
                    return Ok(RebuildProgress::Paused);
                }
                let staged = self
                    .db
                    .stage_rebuild_document(
                        target,
                        document,
                        &chunks,
                        fence.item_id(),
                        fence.token(),
                        &attempt.fingerprint,
                    )
                    .await?;
                if !staged {
                    return Ok(RebuildProgress::Cancelled);
                }
                stream.staged_document_id = Some(document.document_id);
                continue;
            };
            if next_original == Some(document_id)
                && let Some(progress) = self
                    .embed_staged_originals(attempt, fence, document_id, &mut stream)
                    .await?
            {
                return Ok(progress);
            }
            if next_translation == Some(document_id)
                && let Some(progress) = self
                    .embed_staged_translations(attempt, fence, document_id, &mut stream)
                    .await?
            {
                return Ok(progress);
            }
        }
        // A pause requested during the last page still parks before completion:
        // the resumed attempt then completes from the committed cursors.
        let barrier = self.db.read_rebuild_barrier().await?;
        if !rebuild_is_owned(attempt, &fence, &barrier) {
            return Ok(RebuildProgress::Cancelled);
        }
        if barrier.pause_requested {
            return Ok(RebuildProgress::Paused);
        }

        let rebuilt = stream.processed();
        info!(
            chunks_rebuilt = rebuilt,
            "reindexed qdrant collection from app db"
        );
        Ok(RebuildProgress::Completed(rebuilt))
    }

    /// Rebuilds the staged original chunks of one document.
    ///
    /// Every page is embedded, written to the target and checkpointed before
    /// the next page, so a restart replays at most the page that was written but
    /// never committed. Returns a terminal progress only when the attempt must
    /// park or stop; `None` means this document has no original chunk left.
    async fn embed_staged_originals(
        &self,
        attempt: &RebuildAttempt,
        fence: TaskWriteFence,
        document_id: i64,
        stream: &mut RebuildStream,
    ) -> Result<Option<RebuildProgress>> {
        let target = attempt.snapshot.collection.as_str();
        loop {
            // The durable barrier is the authority before every page: a pause
            // request parks after the page this attempt already committed, and
            // a cancelled or superseded owner stops before the next write.
            let barrier = self.db.read_rebuild_barrier().await?;
            if !rebuild_is_owned(attempt, &fence, &barrier) {
                return Ok(Some(RebuildProgress::Cancelled));
            }
            if barrier.pause_requested {
                return Ok(Some(RebuildProgress::Paused));
            }
            let rows = self
                .db
                .list_rebuild_staging_chunks_page(
                    target,
                    stream.original_cursor.map(|cursor| cursor.document_id),
                    stream.original_cursor.map(|cursor| cursor.chunk_index),
                    Self::REINDEX_BATCH_SIZE as i64,
                )
                .await?
                .into_iter()
                .filter(|row| row.document_id == document_id)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                return Ok(None);
            }
            let payloads = rows.iter().map(staged_original_payload).collect::<Vec<_>>();
            let texts = payloads
                .iter()
                .map(|payload| payload.chunk_text.clone())
                .collect::<Vec<_>>();
            let embeddings = attempt.snapshot.embedding.embed_texts(&texts).await?;
            attempt
                .snapshot
                .index
                .replace_document_chunks(&[], &payloads, &embeddings)
                .await?;
            stream.originals += rows.len();
            let last = rows.last().expect("non-empty staged page");
            let mut connection = self.db.pool().acquire().await?;
            let advanced = self
                .db
                .advance_rebuild_cursor_in(
                    &mut connection,
                    fence.item_id(),
                    fence.token(),
                    &attempt.fingerprint,
                    target,
                    crate::db::RebuildCheckpoint {
                        cursor: crate::db::RebuildCursor {
                            document_id: last.document_id,
                            chunk_index: last.chunk_index,
                        },
                        processed_chunks: i64::try_from(stream.originals)
                            .context("the rebuilt original count must fit in BIGINT")?,
                        total_chunks: i64::try_from(stream.reported_total())
                            .context("the total count must fit in BIGINT")?,
                    },
                )
                .await?;
            if !advanced {
                return Ok(Some(RebuildProgress::Cancelled));
            }
            stream.original_cursor = Some(crate::db::RebuildCursor {
                document_id: last.document_id,
                chunk_index: last.chunk_index,
            });
            self.report_rebuild_progress(attempt.generation, stream)
                .await;
            info!(
                batch_size = rows.len(),
                rebuilt = stream.originals,
                document_id,
                "vector rebuild original page completed"
            );
        }
    }

    /// Rebuilds the staged current translations of one document.
    ///
    /// Translation pages have their own cursor because they are ordered by
    /// version as well as document, but they are drained under the same
    /// per-document ordering as the originals.
    async fn embed_staged_translations(
        &self,
        attempt: &RebuildAttempt,
        fence: TaskWriteFence,
        document_id: i64,
        stream: &mut RebuildStream,
    ) -> Result<Option<RebuildProgress>> {
        let target = attempt.snapshot.collection.as_str();
        loop {
            let barrier = self.db.read_rebuild_barrier().await?;
            if !rebuild_is_owned(attempt, &fence, &barrier) {
                return Ok(Some(RebuildProgress::Cancelled));
            }
            if barrier.pause_requested {
                return Ok(Some(RebuildProgress::Paused));
            }
            let rows = self
                .db
                .list_translation_rebuild_staging_page(
                    target,
                    stream.translation_cursor.map(|cursor| cursor.document_id),
                    stream
                        .translation_cursor
                        .map(|cursor| cursor.translation_id),
                    stream.translation_cursor.map(|cursor| cursor.chunk_index),
                    Self::REINDEX_BATCH_SIZE as i64,
                )
                .await?
                .into_iter()
                .filter(|row| row.document_id == document_id)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                return Ok(None);
            }
            let payloads = rows
                .iter()
                .map(staged_translation_payload)
                .collect::<Vec<_>>();
            let texts = payloads
                .iter()
                .map(|payload| payload.chunk_text.clone())
                .collect::<Vec<_>>();
            let embeddings = attempt.snapshot.embedding.embed_texts(&texts).await?;
            attempt
                .snapshot
                .index
                .replace_document_chunks(&[], &payloads, &embeddings)
                .await?;
            stream.translations += rows.len();
            let last = rows.last().expect("non-empty staged translation page");
            let mut connection = self.db.pool().acquire().await?;
            let advanced = self
                .db
                .advance_rebuild_translation_cursor_in(
                    &mut connection,
                    fence.item_id(),
                    fence.token(),
                    (attempt.fingerprint.as_str(), target),
                    crate::db::RebuildTranslationCursor {
                        document_id: last.document_id,
                        translation_id: last.translation_id,
                        chunk_index: last.chunk_index,
                    },
                    i64::try_from(stream.translations)
                        .context("the rebuilt translation count must fit in BIGINT")?,
                )
                .await?;
            if !advanced {
                return Ok(Some(RebuildProgress::Cancelled));
            }
            stream.translation_cursor = Some(crate::db::RebuildTranslationCursor {
                document_id: last.document_id,
                translation_id: last.translation_id,
                chunk_index: last.chunk_index,
            });
            self.report_rebuild_progress(attempt.generation, stream)
                .await;
            info!(
                batch_size = rows.len(),
                rebuilt = stream.translations,
                document_id,
                "vector rebuild translation page completed"
            );
        }
    }

    /// Installs the verified staged originals into the live chunk table.
    ///
    /// The first call marks the durable derived index invalid, removes the
    /// provably disposable old original registry entries in bounded batches,
    /// then resets only `document_chunks` and pins the source total. Every
    /// later page commits its inserted rows, its per-file reservation
    /// increments, the install cursor, the installed count and the epoch in
    /// one owner-fenced transaction. A resumed attempt continues from its
    /// committed install cursor without re-embedding.
    async fn install_rebuilt_originals(
        &self,
        attempt: &RebuildAttempt,
        fence: TaskWriteFence,
        execution: &crate::db::ExecutionContext,
    ) -> Result<RebuildInstallOutcome> {
        let target = attempt.snapshot.collection.as_str();
        let barrier = self.db.read_rebuild_barrier().await?;
        if !rebuild_is_owned(attempt, &fence, &barrier) {
            return Ok(RebuildInstallOutcome::Cancelled);
        }
        if barrier.pause_requested {
            return Ok(RebuildInstallOutcome::Paused);
        }
        if !barrier.install_started {
            // The invalidity must be durable before the first destructive
            // mutation, so a cancellation after it keeps search unavailable.
            let mut write = crate::library_store::begin_task_write(&self.db, fence).await?;
            self.db
                .lock_search_registry_epoch_in(write.connection())
                .await?;
            let barrier = self.db.read_rebuild_barrier_in(write.connection()).await?;
            if !rebuild_is_owned(attempt, &fence, &barrier) {
                write.rollback().await;
                return Ok(RebuildInstallOutcome::Cancelled);
            }
            if barrier.pause_requested {
                write.rollback().await;
                return Ok(RebuildInstallOutcome::Paused);
            }
            self.db
                .mark_document_index_invalid_in(
                    write.connection(),
                    fence.item_id(),
                    fence.token(),
                    &attempt.fingerprint,
                    target,
                )
                .await?;
            if !write.commit().await? {
                return Ok(RebuildInstallOutcome::Cancelled);
            }
            // Old original entries are only provable while the old chunk rows
            // still exist, so this loop runs before the reset.
            loop {
                let mut write = crate::library_store::begin_task_write(&self.db, fence).await?;
                self.db
                    .lock_search_registry_epoch_in(write.connection())
                    .await?;
                let barrier = self.db.read_rebuild_barrier_in(write.connection()).await?;
                if !rebuild_is_owned(attempt, &fence, &barrier) {
                    write.rollback().await;
                    return Ok(RebuildInstallOutcome::Cancelled);
                }
                if barrier.pause_requested {
                    write.rollback().await;
                    return Ok(RebuildInstallOutcome::Paused);
                }
                let deleted = self
                    .db
                    .delete_original_registry_entries_batch_in(
                        write.connection(),
                        REBUILD_ORIGINAL_CLEANUP_BATCH_SIZE,
                    )
                    .await?;
                if deleted > 0 {
                    self.db
                        .bump_search_registry_epoch_in(write.connection())
                        .await?;
                }
                if !write.commit().await? {
                    return Ok(RebuildInstallOutcome::Cancelled);
                }
                if deleted
                    < u64::try_from(REBUILD_ORIGINAL_CLEANUP_BATCH_SIZE)
                        .context("the cleanup batch size must fit in u64")?
                {
                    break;
                }
            }
            // One-time reset of the derived originals, under a short lock
            // timeout. A timeout rolls this transaction back; there is no
            // cascading or broad-delete fallback.
            let mut write = crate::library_store::begin_task_write(&self.db, fence).await?;
            self.db
                .lock_search_registry_epoch_in(write.connection())
                .await?;
            let barrier = self.db.read_rebuild_barrier_in(write.connection()).await?;
            if !rebuild_is_owned(attempt, &fence, &barrier) {
                write.rollback().await;
                return Ok(RebuildInstallOutcome::Cancelled);
            }
            if barrier.pause_requested {
                write.rollback().await;
                return Ok(RebuildInstallOutcome::Paused);
            }
            if barrier.install_started {
                write.rollback().await;
            } else {
                let staging = self
                    .db
                    .read_rebuild_staging_state_in(write.connection())
                    .await?;
                if !staging.staging_complete {
                    anyhow::bail!(
                        "the bounded original installation requires a complete staged snapshot"
                    );
                }
                let verification = self
                    .db
                    .verify_rebuild_staging_in(write.connection(), target)
                    .await?;
                if verification.document_hash_mismatch != 0
                    || verification.chunk_count_mismatch != 0
                    || verification.chunk_hash_mismatch != 0
                    || verification.document_membership_mismatch != 0
                {
                    anyhow::bail!(
                        "the staged original snapshot changed before the reset; retry the rebuild"
                    );
                }
                self.db
                    .set_rebuild_install_lock_timeout_in(write.connection(), "1s")
                    .await?;
                self.db
                    .truncate_document_chunks_in(write.connection())
                    .await?;
                let total = self
                    .db
                    .begin_rebuild_install_in(
                        write.connection(),
                        fence.item_id(),
                        fence.token(),
                        &attempt.fingerprint,
                        target,
                    )
                    .await?;
                if total.is_none() {
                    write.rollback().await;
                    return Ok(RebuildInstallOutcome::Cancelled);
                }
                if !write.commit().await? {
                    return Ok(RebuildInstallOutcome::Cancelled);
                }
            }
        }
        self.run_install_batches(attempt, fence, execution).await
    }

    /// Commits the staged originals in bounded owner-fenced batches.
    async fn run_install_batches(
        &self,
        attempt: &RebuildAttempt,
        fence: TaskWriteFence,
        execution: &crate::db::ExecutionContext,
    ) -> Result<RebuildInstallOutcome> {
        let target = attempt.snapshot.collection.as_str();
        loop {
            // The previous page's item lock is released before this point. The
            // authority is renewed in its own short transaction, and the page
            // opens with exactly the opaque proof of that renewal: a busy
            // background mirror is not reused as a fresh page fence, and no
            // library lock is taken while the page holds item/epoch locks.
            let Some(lease_proof) = self.db.renew_task_item_lease(execution).await? else {
                tracing::warn!(
                    target: "task_lifecycle",
                    task_id = %execution.task_id(),
                    item_id = %execution.item_id,
                    attempt_id = %execution.attempt_id(),
                    "task item lease was lost before a rebuild install page"
                );
                return Ok(RebuildInstallOutcome::ClaimLost);
            };
            let mut write = match crate::library_store::begin_task_write_with_lease(
                &self.db,
                fence,
                lease_proof,
            )
            .await
            {
                Ok(write) => write,
                Err(error)
                    if error
                        .downcast_ref::<crate::library_store::TaskWriteDenied>()
                        .is_some() =>
                {
                    tracing::warn!(
                        target: "task_lifecycle",
                        task_id = %execution.task_id(),
                        item_id = %execution.item_id,
                        attempt_id = %execution.attempt_id(),
                        "task item lease was lost before a rebuild install page"
                    );
                    return Ok(RebuildInstallOutcome::ClaimLost);
                }
                Err(error) => return Err(error),
            };
            self.db
                .lock_search_registry_epoch_in(write.connection())
                .await?;
            let barrier = self.db.read_rebuild_barrier_in(write.connection()).await?;
            if !rebuild_is_owned(attempt, &fence, &barrier) {
                write.rollback().await;
                return Ok(RebuildInstallOutcome::Cancelled);
            }
            if barrier.pause_requested {
                write.rollback().await;
                return Ok(RebuildInstallOutcome::Paused);
            }
            if !barrier.install_started {
                anyhow::bail!("the bounded original installation has not started");
            }
            let rows = self
                .db
                .list_rebuild_install_chunk_page_in(
                    write.connection(),
                    target,
                    barrier.install_cursor.as_ref(),
                    REBUILD_INSTALL_BATCH_SIZE,
                )
                .await?;
            if rows.is_empty() {
                install_completion(barrier.installed_chunks, barrier.install_total_chunks, true)?;
                // The empty final page still holds the page's fenced grant and
                // the fresh mirrored deadline. Commit it like every other page
                // instead of rolling the renewed mirror back: a page that
                // releases its lease here would let recovery park an otherwise
                // finished install. A commit the fence no longer authorizes is
                // a typed claim loss, not a rolled-back success.
                if !write.commit().await? {
                    return Ok(RebuildInstallOutcome::ClaimLost);
                }
                return Ok(RebuildInstallOutcome::Installed);
            }
            let count = i64::try_from(rows.len()).context("the installed page must fit i64")?;
            let expected = barrier
                .installed_chunks
                .checked_add(count)
                .context("the installed count overflowed")?;
            if expected > barrier.install_total_chunks {
                anyhow::bail!(
                    "the staged original source exceeds its pinned total of {}",
                    barrier.install_total_chunks
                );
            }
            let last = rows.last().expect("a non-empty install page");
            let up_to = crate::db::RebuildCursor {
                document_id: last.document_id,
                chunk_index: last.chunk_index,
            };
            self.db
                .insert_rebuild_install_chunk_page_in(
                    write.connection(),
                    target,
                    barrier.install_cursor.as_ref(),
                    &up_to,
                )
                .await?;
            let mismatch = self
                .db
                .prove_rebuild_install_chunk_page_in(
                    write.connection(),
                    target,
                    barrier.install_cursor.as_ref(),
                    &up_to,
                )
                .await?;
            if mismatch != 0 {
                anyhow::bail!(
                    "the installed original page does not match its staged source ({mismatch} rows)"
                );
            }
            let groups = self
                .db
                .rebuild_install_page_file_counts_in(
                    write.connection(),
                    target,
                    barrier.install_cursor.as_ref(),
                    &up_to,
                )
                .await?;
            let grouped = groups.iter().try_fold(0_i64, |total, group| {
                total
                    .checked_add(group.count)
                    .context("a file count overflowed")
            })?;
            if grouped != count {
                anyhow::bail!(
                    "the per-file page counts {grouped} do not equal the installed page {count}"
                );
            }
            for group in groups {
                let run_id = original_reservation_run_id(target, group.file_id);
                let reserved = self
                    .db
                    .upsert_original_reservation_in(
                        write.connection(),
                        crate::db::OriginalRebuildReservation {
                            run_id,
                            file_id: group.file_id,
                            generation: attempt.rebuild_generation,
                            count: group.count,
                            fingerprint: &attempt.fingerprint,
                            collection: target,
                        },
                    )
                    .await?;
                if !reserved {
                    anyhow::bail!(
                        "the original reservation of the installed page has an unexpected shape"
                    );
                }
            }
            let advanced = self
                .db
                .advance_rebuild_install_in(
                    write.connection(),
                    fence.item_id(),
                    fence.token(),
                    &attempt.fingerprint,
                    target,
                    crate::db::RebuildInstallAdvance {
                        cursor: up_to,
                        installed: count,
                        expected_installed: barrier.installed_chunks,
                    },
                )
                .await?;
            if advanced.is_none() {
                write.rollback().await;
                return Ok(RebuildInstallOutcome::Cancelled);
            }
            if !write.commit().await? {
                return Ok(RebuildInstallOutcome::Cancelled);
            }
        }
    }

    /// Chunks one canonical body off the async worker threads.
    ///
    /// The closure only computes: it owns the normalized document, the pinned
    /// budget and the chunk-id seed, and returns the chunks. Dropping it cannot
    /// stop a closure that already started, so the caller rechecks ownership and
    /// pause after it returns and discards the result before any staging write.
    async fn chunk_rebuild_document(
        &self,
        attempt: &RebuildAttempt,
        document: &RebuildDocumentRow,
    ) -> Result<Vec<DocumentChunk>> {
        let normalized = rebuild_normalized_document(document);
        let budget = attempt.chunk_budget.clone();
        let document_id = document.document_id;
        let source_key = document.source_key.clone();
        let generation = attempt.rebuild_generation;
        tokio::task::spawn_blocking(move || {
            chunk_document_iter_resolved(
                document_id,
                &source_key,
                &normalized,
                &budget,
                Some(generation),
            )
            .map_err(|error| anyhow::anyhow!("chunk rebuild document: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| anyhow::anyhow!("chunk rebuild document: {error}"))
        })
        .await
        .context("the rebuild chunk task did not finish")?
    }

    /// The exact number of staged chunks of one target.
    ///
    /// The original and translation counters keep their distinct meanings; this
    /// is the total that the completion guard and the final verification need.
    async fn count_staged_chunks(&self, target: &str) -> Result<usize> {
        let counts = self.db.count_rebuild_staging_chunks(target).await?;
        usize::try_from(
            counts
                .original
                .checked_add(counts.translated)
                .context("the staged chunk total overflowed")?,
        )
        .context("the staged chunk total must be non-negative")
    }

    /// Reports the streamed position until the exact staged total is known.
    async fn report_rebuild_progress(&self, generation: u64, stream: &RebuildStream) {
        self.set_vector_rebuild_progress(generation, stream.processed(), stream.reported_total())
            .await;
    }

    pub async fn vector_index_rebuild_status(&self) -> VectorIndexRebuildStatus {
        self.vector_rebuild_status.status()
    }

    pub(crate) async fn run_vector_index_rebuild(
        &self,
        fence: TaskWriteFence,
        execution: crate::db::ExecutionContext,
        task_id: Uuid,
        attempt_id: i64,
    ) -> Result<super::RebuildRunOutcome> {
        // A process can die after the durable pause committed but before the
        // worker parked the item on the paused dependency. The reclaimed item
        // is the same owner, so it must park again instead of claiming the
        // paused barrier as a busy qdrant wait. It never resumes on its own:
        // only the explicit resume clears the flag and reopens the item.
        let barrier = self.db.read_rebuild_barrier().await?;
        if paused_rebuild_owned_by(&barrier, fence.item_id()) {
            return Ok(super::RebuildRunOutcome::Paused);
        }
        let attempt = self.begin_vector_index_rebuild(fence).await?;
        let attempt_guard = super::RebuildAttemptGuard::new(
            &self.vector_rebuild_status,
            &self.vector,
            attempt.generation,
        );
        let progress = match self.rebuild_index_with_snapshot(&attempt, fence).await {
            Ok(progress) => progress,
            Err(error) => {
                self.fail_rebuild_barrier(fence, &error).await;
                attempt_guard.disarm();
                fail_rebuild_attempt(&self.vector, &self.vector_rebuild_status, &attempt, error)
                    .await?;
                unreachable!("a failed rebuild attempt returns its error");
            }
        };
        match progress {
            RebuildProgress::Paused => {
                // A paused attempt is not a failure: the durable barrier keeps
                // its owner and cursor, and only the in-process admission is
                // released so the explicit resume can admit the next attempt.
                self.vector.abandon_rebuild(attempt.generation);
                self.vector_rebuild_status
                    .fail(attempt.generation, "paused".to_string());
                attempt_guard.disarm();
                Ok(super::RebuildRunOutcome::Paused)
            }
            RebuildProgress::Cancelled => {
                // The barrier was cancelled or taken over. Closing the local
                // fence without publishing anything is the whole outcome: the
                // cancelled work must never latch a failure or clear a barrier
                // that no longer belongs to it.
                self.vector.abandon_rebuild(attempt.generation);
                self.vector_rebuild_status
                    .fail(attempt.generation, "cancelled".to_string());
                attempt_guard.disarm();
                Ok(super::RebuildRunOutcome::Cancelled)
            }
            RebuildProgress::Completed(rebuilt) => {
                // The target is verified before anything is activated: it must
                // exist, declare the configured dimension, and hold exactly the
                // chunks this build committed. An unverified target is never
                // activated, and the old active collection keeps serving.
                if let Err(error) = self.verify_rebuild_target(&attempt, rebuilt).await {
                    self.fail_rebuild_barrier(fence, &error).await;
                    attempt_guard.disarm();
                    fail_rebuild_attempt(
                        &self.vector,
                        &self.vector_rebuild_status,
                        &attempt,
                        error,
                    )
                    .await?;
                    unreachable!("a failed rebuild attempt returns its error");
                }
                // The verified target now receives the verified staged
                // originals in bounded batches. A pause or cancellation keeps
                // the committed install cursor and the durable invalidity, so
                // the resumed attempt continues without re-embedding and the
                // partially replaced chunk table never becomes searchable.
                match self
                    .install_rebuilt_originals(&attempt, fence, &execution)
                    .await
                {
                    Ok(RebuildInstallOutcome::Installed) => {}
                    Ok(RebuildInstallOutcome::Paused) => {
                        self.vector.abandon_rebuild(attempt.generation);
                        self.vector_rebuild_status
                            .fail(attempt.generation, "paused".to_string());
                        attempt_guard.disarm();
                        return Ok(super::RebuildRunOutcome::Paused);
                    }
                    Ok(RebuildInstallOutcome::Cancelled) => {
                        self.vector.abandon_rebuild(attempt.generation);
                        self.vector_rebuild_status
                            .fail(attempt.generation, "cancelled".to_string());
                        attempt_guard.disarm();
                        return Ok(super::RebuildRunOutcome::Cancelled);
                    }
                    // The page boundary found the lease gone. The local attempt
                    // stops, but the durable barrier is not written through the
                    // stale owner: the outcome is a typed claim loss the driver
                    // handles without failure bookkeeping.
                    Ok(RebuildInstallOutcome::ClaimLost) => {
                        self.vector.abandon_rebuild(attempt.generation);
                        self.vector_rebuild_status
                            .fail(attempt.generation, "claim lost".to_string());
                        attempt_guard.disarm();
                        return Ok(super::RebuildRunOutcome::ClaimLost);
                    }
                    Err(error) => {
                        self.fail_rebuild_barrier(fence, &error).await;
                        attempt_guard.disarm();
                        fail_rebuild_attempt(
                            &self.vector,
                            &self.vector_rebuild_status,
                            &attempt,
                            error,
                        )
                        .await?;
                        unreachable!("a failed rebuild attempt returns its error");
                    }
                }
                // The installation can pause, cancel, or fail, but a completed
                // installation must still be proven before publication: the
                // target is re-inspected after the last install write, so a
                // count or dimension drift during installation blocks the
                // transaction with the invalidity and old active untouched.
                if let Err(error) = self.verify_rebuild_target(&attempt, rebuilt).await {
                    self.fail_rebuild_barrier(fence, &error).await;
                    attempt_guard.disarm();
                    fail_rebuild_attempt(
                        &self.vector,
                        &self.vector_rebuild_status,
                        &attempt,
                        error,
                    )
                    .await?;
                    unreachable!("a failed rebuild attempt returns its error");
                }
                // The writer seam receives the identity the attempt actually
                // re-embedded with, including the real completed count. The
                // production closure writes the identity, activates the target,
                // clears the barrier and moves the epochs in one transaction,
                // so the identity of a finished rebuild and the barrier that
                // describes it can never disagree.
                let db = self.db.clone();
                let execution = execution.clone();
                let outcome = settle_successful_rebuild(
                    &self.vector,
                    &self.vector_rebuild_status,
                    &attempt,
                    rebuilt,
                    move |identity| {
                        Box::pin(async move {
                            complete_rebuild_transaction(
                                &db, fence, &execution, task_id, attempt_id, &identity,
                            )
                            .await
                        })
                    },
                )
                .await;
                if let Err(error) = &outcome {
                    self.fail_rebuild_barrier(fence, error).await;
                }
                attempt_guard.disarm();
                outcome.map(|_| super::RebuildRunOutcome::Completed)
            }
        }
    }

    /// Records a failed rebuild in the persistent barrier.
    ///
    /// The barrier keeps its owner and its identity, so a retry of the same
    /// identity may take it over again. The write is owner-guarded, so an
    /// attempt that was superseded never touches the barrier of its
    /// successor.
    async fn fail_rebuild_barrier(&self, fence: TaskWriteFence, error: &anyhow::Error) {
        let mut connection = match self.db.pool().acquire().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "failed to open the rebuild failure transaction");
                return;
            }
        };
        // The epoch row is locked before it is written, so the failure record
        // takes the same order as every other writer of that row.
        if let Err(error) = self.db.lock_search_registry_epoch_in(&mut connection).await {
            tracing::warn!(%error, "failed to lock the rebuild barrier for the failure record");
            return;
        }
        let recorded = self
            .db
            .fail_rebuild_in(
                &mut connection,
                fence.item_id(),
                fence.token(),
                &error.to_string(),
            )
            .await;
        if let Err(error) = recorded {
            tracing::warn!(%error, "failed to record the failed rebuild barrier");
        }
    }

    /// Admits an explicit rebuild through the runtime's one atomic admission.
    ///
    /// The rebuild always writes a fresh physical target: the desired
    /// configuration is resolved from the persisted settings, the provider is
    /// proven with one synthetic embedding, and the target is named and
    /// recorded in the durable barrier before any network work happens. An
    /// interrupted build of the same identity resumes the target the barrier
    /// already records, with its committed cursor, and never derives a new
    /// name for it. The fresh-versus-resume decision is made by the claim
    /// itself, under the epoch lock, so a barrier that became idle between an
    /// earlier read and the claim can never attach a cancelled or previous
    /// target.
    ///
    /// The persistent barrier is taken first: it is the authority that no
    /// registry-admitted publication overlaps the rebuild, and it outlives the
    /// process, so a node that never observed the rebuild still fails closed.
    /// The barrier is claimed under the captured grant of the task item, so
    /// the owner of the barrier is a live writer that can prove it.
    ///
    /// The runtime then admits the target itself, not the published pair: a
    /// deployment whose active collection is stale, or whose installation
    /// never had one, can still be rebuilt into a fresh target.
    pub(crate) async fn begin_vector_index_rebuild(
        &self,
        fence: TaskWriteFence,
    ) -> Result<RebuildAttempt> {
        let plan = self.resolve_rebuild_target().await?;
        // The target decision is made by the claim below, under the epoch lock
        // and from the barrier row it updates. This name is only the candidate a
        // fresh claim would write; an owned resume replaces it with the target
        // the barrier records.
        let fresh_collection = fresh_target_name(&plan.base_collection, &plan.fingerprint);

        let mut write = crate::library_store::begin_task_write(&self.db, fence).await?;
        // The persisted configuration is the authority of the vector space, and
        // the begin kernel compares it with the captured identity after it
        // holds the registry epoch. A settings save therefore either happens
        // entirely before the comparison or entirely after the barrier claim,
        // so a rebuild never re-embeds an old provider under a newer identity.
        // The fresh-versus-resume decision is part of the same locked claim.
        let (outcome, resumed) = self
            .db
            .begin_rebuild_checked_with_budget_in(
                write.connection(),
                fence.item_id(),
                fence.token(),
                &plan.fingerprint,
                &fresh_collection,
                None,
            )
            .await?;
        let staging = self
            .db
            .read_rebuild_staging_state_in(write.connection())
            .await?;
        if !write.commit().await? {
            return Err(anyhow::anyhow!(
                "the grant no longer authorizes the rebuild start"
            ));
        }
        let barrier = match outcome {
            crate::db::RebuildStartOutcome::Started(barrier) => *barrier,
            crate::db::RebuildStartOutcome::Refused(reason) => {
                return Err(rebuild_refusal_error(reason));
            }
        };
        // Retained across the provider and Qdrant awaits: the decision this
        // attempt made under the claim is never recomputed from a later read.
        let collection = barrier.collection.clone().ok_or_else(|| {
            anyhow::anyhow!("the rebuild barrier claimed a start without a target collection")
        })?;
        // A fresh claim persisted the latest saved budget under the epoch lock,
        // and a resume reuses the snapshot its build pinned. A barrier without
        // both values predates durable staging and cannot proceed.
        let pinned_value = staging.chunk_budget.as_ref().ok_or_else(|| {
            anyhow::anyhow!("the rebuild barrier recorded no pinned chunk budget")
        })?;
        let pinned_budget = parse_chunk_budget_value(pinned_value)
            .map_err(|error| anyhow::anyhow!("parse the pinned chunk budget: {error}"))?;
        let chunk_budget = self
            .chunking
            .resolve(&pinned_budget)
            .map_err(|error| anyhow::anyhow!("resolve the pinned chunk budget: {error}"))?;
        let rebuild_generation = staging.generation.ok_or_else(|| {
            anyhow::anyhow!("the rebuild barrier recorded no pinned rebuild generation")
        })?;
        if !self.vector_rebuild_status.claim_start() {
            self.release_rebuild_barrier(fence).await;
            return Err(RebuildBusy.into());
        }
        // The reservation releases the tracker gate on every later return or
        // error, so a cancelled or failed start can never block the next
        // explicit rebuild until the process restarts.
        let start_reservation = RebuildStartReservation {
            tracker: &self.vector_rebuild_status,
            armed: true,
        };
        // The provider is proven before a target is written: one synthetic
        // document embedding, and a query embedding only when the query model
        // differs from the document model. The vectors must have the
        // configured dimension and be finite, so even a rebuild without a
        // single chunk proves that the provider produces the vectors the
        // target stores.
        if let Err(error) = plan.prove_provider().await {
            // The barrier keeps this owner and the recorded target, so the
            // failed build can be retried on the same target.
            self.fail_rebuild_barrier(fence, &error).await;
            return Err(error);
        }
        // A cancellation or an expired lease can win while the provider probe
        // is in flight. The captured grant and the exact durable barrier are
        // rechecked before target preparation can start another Qdrant
        // mutation; an already accepted provider request is left to finish.
        if let Err(error) = self
            .assert_rebuild_target_authority(fence, &plan.fingerprint, &collection)
            .await
        {
            self.release_rebuild_barrier(fence).await;
            return Err(error);
        }
        // The target is prepared after the barrier records its name, so a crash
        // leaves a recorded target instead of an unnamed collection. The
        // durable ownership marker of that target decides what may happen to
        // it: a target this attempt created is attached, an unmarked target
        // that already exists is ambiguous and is never adopted, and an absent
        // target is created. Only the decision made under the claim decides
        // this, and the recorded target is never the currently active physical
        // collection.
        if resumed
            && let Some(active) = self.db.read_active_vector_index_state().await?
            && active.collection_name == collection
        {
            self.fail_rebuild_barrier(
                fence,
                &anyhow::anyhow!(
                    "the recorded rebuild target is the active collection; it is never rebuilt in \
                     place"
                ),
            )
            .await;
            return Err(DomainError::conflict(
                "the recorded rebuild target is the active collection; it is never rebuilt in place",
            )
            .into());
        }
        let prepared = QdrantIndex::prepare_rebuild_target(
            &plan.qdrant,
            &collection,
            plan.dimensions,
            barrier.rebuild_target_created,
            || self.assert_rebuild_target_authority(fence, &plan.fingerprint, &collection),
        )
        .await;
        let (index, created) = match prepared {
            Ok(prepared) => prepared,
            Err(error) if QdrantIndex::is_target_collision(&error) => {
                // The recorded target is not provably the collection of this
                // attempt: it is never attached and never written. The own
                // barrier is released instead, so the next explicit attempt
                // derives a fresh target name and the refused collection
                // stays untouched.
                self.clear_refused_rebuild_target(fence, &collection).await;
                return Err(DomainError::conflict(error.to_string()).into());
            }
            Err(error) => {
                // The barrier keeps this owner and the recorded target, so the
                // failed build can be retried on the same target.
                self.fail_rebuild_barrier(fence, &error).await;
                return Err(error);
            }
        };
        if created {
            // The create is recorded under the exact owner before any page is
            // written, so a later attempt may attach this very collection
            // instead of mistaking it for a foreign one.
            let mut write = crate::library_store::begin_task_write(&self.db, fence).await?;
            let marked = self
                .db
                .mark_rebuild_target_created_in(
                    write.connection(),
                    fence.item_id(),
                    fence.token(),
                    &plan.fingerprint,
                    &collection,
                )
                .await?;
            if !write.commit().await? {
                self.clear_refused_rebuild_target(fence, &collection).await;
                return Err(anyhow::anyhow!(
                    "the grant no longer authorizes the rebuild target"
                ));
            }
            if !marked {
                self.clear_refused_rebuild_target(fence, &collection).await;
                return Err(anyhow::anyhow!(
                    "the rebuild target is no longer owned by this attempt"
                ));
            }
        }
        let snapshot = VectorSnapshot {
            embedding: plan.provider.clone(),
            identity: Some(plan.identity.clone()),
            index: Arc::new(index),
            collection: collection.clone(),
            generation: 0,
            configured_fingerprint: Some(plan.fingerprint.clone()),
        };
        let admitted = match self
            .vector
            .try_admit_target_rebuild(snapshot, plan.fingerprint.clone())
        {
            Ok(admitted) => admitted,
            Err(error) => {
                self.release_rebuild_barrier(fence).await;
                return Err(match error {
                    RebuildAdmissionError::NotConfigured
                    | RebuildAdmissionError::MissingIdentity
                    | RebuildAdmissionError::MissingFingerprint => {
                        DomainError::unavailable(error.message()).into()
                    }
                    RebuildAdmissionError::Busy => return Err(RebuildBusy.into()),
                    RebuildAdmissionError::StaleSnapshot => {
                        DomainError::conflict(error.message()).into()
                    }
                });
            }
        };
        self.vector_rebuild_status.begin(admitted.generation);
        start_reservation.settle();
        Ok(RebuildAttempt {
            generation: admitted.generation,
            snapshot: admitted.snapshot,
            fingerprint: admitted.configured_fingerprint,
            qdrant: plan.qdrant,
            base_collection: plan.base_collection,
            dimensions: plan.dimensions,
            chunk_budget,
            rebuild_generation,
        })
    }

    /// Resolves the desired vector identity one rebuild target is built from.
    ///
    /// The persisted runtime settings are the authority: the rebuild builds
    /// the space the deployment is configured for, never the space the active
    /// collection happens to hold. The API key comes from the shared store, so
    /// the target is embedded with the credential this deployment owns.
    async fn resolve_rebuild_target(&self) -> Result<RebuildTargetPlan> {
        let stored = self
            .db
            .get_runtime_settings()
            .await?
            .ok_or_else(|| -> anyhow::Error {
                DomainError::unavailable(
                    "the runtime settings are not configured; the rebuild cannot know which vector \
                 space to build",
                )
                .into()
            })?;
        let api_key = crate::services::settings::secrets::resolve_stored(
            &self.store,
            crate::services::secret_store::SecretPurpose::EmbeddingApiKey,
            crate::services::secret_store::key_names::EMBEDDING_API_KEY,
        )
        .await?;
        let embedding = crate::config::EmbeddingConfig {
            base_url: stored.embedding.base_url.trim().to_string(),
            api_key,
            model: stored.embedding.model.trim().to_string(),
            query_model: stored.embedding.query_model.clone(),
            dimensions: stored.embedding.dimensions,
            timeout: Duration::from_secs(stored.embedding.timeout_secs),
        };
        let provider = build_provider(&embedding)?;
        let identity = EmbeddingIdentity::from_config(&embedding);
        let fingerprint = vector_identity_fingerprint(
            stored.qdrant.url.trim(),
            stored.qdrant.collection_name.trim(),
            stored.embedding.base_url.trim(),
            stored.embedding.model.trim(),
            stored.embedding.dimensions,
        );
        Ok(RebuildTargetPlan {
            provider,
            identity,
            fingerprint,
            qdrant: QdrantConfig {
                url: stored.qdrant.url.trim().to_string(),
                collection_name: stored.qdrant.collection_name.trim().to_string(),
                recreate_on_dimension_mismatch: false,
            },
            base_collection: stored.qdrant.collection_name.trim().to_string(),
            dimensions: stored.embedding.dimensions,
        })
    }

    /// Verifies the target before it may be activated.
    ///
    /// The declared dimension and the exact point count are the proof that the
    /// target holds the vectors this build wrote: a target of another
    /// dimension is a different vector space, and a count that differs from
    /// the committed chunk count describes an incomplete build.
    async fn verify_rebuild_target(&self, attempt: &RebuildAttempt, rebuilt: usize) -> Result<()> {
        let observed = QdrantIndex::inspect_collection(
            &attempt.qdrant,
            &attempt.snapshot.collection,
            attempt.dimensions,
        )
        .await?;
        let Some((points, dimensions)) = observed else {
            return Err(anyhow::anyhow!(
                "the verified rebuild target {} does not exist",
                attempt.snapshot.collection
            ));
        };
        if dimensions != attempt.dimensions {
            return Err(anyhow::anyhow!(
                "the rebuild target {} declares {dimensions} dimensions, but the configured \
                 identity needs {}",
                attempt.snapshot.collection,
                attempt.dimensions
            ));
        }
        let expected = u64::try_from(rebuilt).context("the rebuilt count must fit in a u64")?;
        if points != expected {
            return Err(anyhow::anyhow!(
                "the rebuild target {} holds {points} points, but the build committed {expected} \
                 chunks",
                attempt.snapshot.collection
            ));
        }
        Ok(())
    }

    /// Re-proves the exact rebuild authority immediately before a remote
    /// mutation.
    ///
    /// The captured grant and the durable barrier are rechecked after every
    /// preceding await. A refusal starts no new Qdrant mutation; it never
    /// claims that a request already accepted by Qdrant was rolled back.
    async fn assert_rebuild_target_authority(
        &self,
        fence: TaskWriteFence,
        fingerprint: &str,
        collection: &str,
    ) -> Result<()> {
        if !crate::library_store::task_write_granted(&self.db, fence).await? {
            return Err(DomainError::conflict(
                "the rebuild grant expired before its target was prepared",
            )
            .into());
        }
        let barrier = self.db.read_rebuild_barrier().await?;
        if !barrier.is_owned_by(fence.item_id(), fence.token())
            || barrier.fingerprint.as_deref() != Some(fingerprint)
            || barrier.collection.as_deref() != Some(collection)
        {
            return Err(DomainError::conflict(
                "the rebuild barrier was cancelled or taken over before its target was prepared",
            )
            .into());
        }
        Ok(())
    }

    /// Returns the barrier to idle after the rebuild never started.
    ///
    /// The rollback is only justified while no rebuild RPC could have
    /// occurred: the in-process admission refused the attempt before any
    /// point was written, so the collection is untouched and the barrier has
    /// nothing to describe. Every failure after that keeps the owned failed
    /// state instead, and the write is owner-guarded, so it never clears the
    /// barrier of another owner.
    async fn release_rebuild_barrier(&self, fence: TaskWriteFence) {
        let mut connection = match self.db.pool().acquire().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "failed to open the rebuild release transaction");
                return;
            }
        };
        if let Err(error) = self.db.lock_search_registry_epoch_in(&mut connection).await {
            tracing::warn!(%error, "failed to lock the rebuild barrier for the release");
            return;
        }
        if let Err(error) = self
            .db
            .release_rebuild_in(&mut connection, fence.item_id(), fence.token())
            .await
        {
            tracing::warn!(%error, "failed to release the rebuild barrier");
        }
    }

    /// Releases the barrier of a rebuild target this attempt may not use.
    ///
    /// The clear is fenced by the owner item, its grant token and the recorded
    /// collection, so it releases the barrier of this attempt and never the
    /// barrier of a newer owner. The refused collection is left untouched in
    /// Qdrant: only the durable record is released, so the next explicit
    /// attempt derives a fresh target name.
    async fn clear_refused_rebuild_target(&self, fence: TaskWriteFence, collection: &str) {
        let mut connection = match self.db.pool().acquire().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "failed to open the rebuild refusal transaction");
                return;
            }
        };
        match self
            .db
            .clear_owned_rebuild_target_in(
                &mut connection,
                fence.item_id(),
                fence.token(),
                collection,
            )
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => {
                tracing::warn!(
                    collection,
                    "the refused rebuild target is no longer owned by this attempt"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "failed to clear the refused rebuild target");
            }
        }
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

/// Where one rebuild attempt stopped.
enum RebuildProgress {
    Completed(usize),
    Paused,
    Cancelled,
}

/// Where the bounded original installation stopped.
enum RebuildInstallOutcome {
    Installed,
    Paused,
    Cancelled,
    ClaimLost,
}

/// The committed streaming position of one rebuild attempt.
///
/// The original and translation cursors stay distinct because their page
/// queries are ordered differently, but both are drained one document at a
/// time: a document's originals are written before its translations, and no
/// later document is staged before the current one is fully written.
struct RebuildStream {
    /// The last canonical document whose fresh chunks were committed.
    staged_document_id: Option<i64>,
    original_cursor: Option<crate::db::RebuildCursor>,
    translation_cursor: Option<crate::db::RebuildTranslationCursor>,
    originals: usize,
    translations: usize,
    /// The exact staged total, once staging has completed.
    known_total: Option<usize>,
}

impl RebuildStream {
    fn processed(&self) -> usize {
        self.originals + self.translations
    }

    /// The total to report and to checkpoint.
    ///
    /// `0` means the final total is not known yet, which the durable counter
    /// permits only while staging is incomplete.
    fn reported_total(&self) -> usize {
        self.known_total.unwrap_or(0)
    }
}

/// Whether the durable barrier still belongs to this attempt.
///
/// Ownership alone is not enough: the captured identity and physical collection
/// are part of the record, so a rebuild that was superseded by a new vector
/// identity stops before its next page.
fn rebuild_is_owned(
    attempt: &RebuildAttempt,
    fence: &TaskWriteFence,
    barrier: &crate::db::RebuildBarrier,
) -> bool {
    barrier.is_owned_by(fence.item_id(), fence.token())
        && barrier.fingerprint.as_deref() == Some(attempt.fingerprint.as_str())
        && barrier.collection.as_deref() == Some(attempt.snapshot.collection.as_str())
}

/// Whether a reclaimed item is the recorded owner of a durably paused rebuild.
///
/// The owner token is deliberately not compared: a restart reclaims the item
/// with a fresh token, while the barrier keeps the token of the dead attempt.
/// The item identity is the durable owner, and the paused flag keeps the
/// rebuild resumable until an explicit resume clears it.
fn paused_rebuild_owned_by(barrier: &crate::db::RebuildBarrier, item_id: Uuid) -> bool {
    barrier.state == crate::db::RebuildBarrierState::Running
        && barrier.pause_requested
        && barrier.owner_item_id == Some(item_id)
}

/// The immutable context one admitted explicit rebuild owns.
pub(crate) struct RebuildAttempt {
    pub(crate) generation: u64,
    pub(crate) snapshot: crate::vector_runtime::VectorSnapshot,
    fingerprint: String,
    /// The destination the target lives in, so the verification can read the
    /// target back without resolving the configuration again.
    qdrant: QdrantConfig,
    /// The logical collection name the identity was built for, never the
    /// physical target. Activation uses it to reconstruct a legacy endpoint
    /// proof.
    base_collection: String,
    /// The dimension the target must declare.
    dimensions: usize,
    /// The chunk budget pinned when this build first took the barrier.
    chunk_budget: ResolvedChunkBudget,
    /// The persisted chunk-id seed of this build.
    rebuild_generation: i64,
}

/// The desired vector identity one rebuild target is built from.
///
/// It is resolved from the persisted runtime settings, so a rebuild builds the
/// space the deployment is configured for and never the space the active
/// collection happens to hold.
struct RebuildTargetPlan {
    provider: Arc<dyn EmbeddingProvider>,
    identity: EmbeddingIdentity,
    /// The logical fingerprint of the desired vector space.
    fingerprint: String,
    qdrant: QdrantConfig,
    /// The logical collection name the settings carry. It names the identity,
    /// not the physical target.
    base_collection: String,
    dimensions: usize,
}

impl RebuildTargetPlan {
    /// Proves the provider before any target is written.
    ///
    /// One synthetic document embedding exercises the document model, and a
    /// query embedding is issued only when the query model differs from it.
    /// The returned vectors must have the configured dimension and hold only
    /// finite values, so even a rebuild without a single chunk proves that
    /// the provider produces the vectors the target stores. Neither the probe
    /// text nor the credential is ever logged.
    async fn prove_provider(&self) -> Result<()> {
        let documents = self
            .provider
            .embed_texts(&["context69 rebuild target probe".to_string()])
            .await?;
        let document = documents.first().ok_or_else(|| {
            anyhow::anyhow!("the embedding provider returned no vector for the rebuild probe")
        })?;
        self.assert_probe_vector(document)?;
        if self
            .provider
            .effective_query_model()
            .is_some_and(|model| model != self.identity.model)
        {
            let query = self
                .provider
                .embed_query("context69 rebuild target query probe")
                .await?;
            self.assert_probe_vector(&query)?;
        }
        Ok(())
    }

    fn assert_probe_vector(&self, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dimensions {
            return Err(anyhow::anyhow!(
                "the embedding provider returned {} dimensions for the rebuild probe, but the \
                 configured identity needs {}",
                vector.len(),
                self.dimensions
            ));
        }
        if !vector.iter().all(|value| value.is_finite()) {
            return Err(anyhow::anyhow!(
                "the embedding provider returned a non-finite value for the rebuild probe"
            ));
        }
        Ok(())
    }
}

/// Builds the chunking input of one stored document without normalizing or
/// rehashing it: the stored record hash is the authority the staging
/// verification and activation compare against.
fn rebuild_normalized_document(row: &RebuildDocumentRow) -> NormalizedDocument {
    NormalizedDocument {
        external_id: row.external_id.clone(),
        title: row.title.clone(),
        summary: row.summary.clone(),
        body_text: row.body_text.clone(),
        source_uri: row.source_uri.clone(),
        published_at: row.published_at,
        updated_at: row.updated_at_source,
        metadata_json: row.metadata_json.clone(),
        record_hash: row.record_hash.clone(),
    }
}

/// The physical name of one fresh rebuild target.
///
/// The name carries the identity it is built for and a unique suffix, so two
/// builds never share a collection and a target stays traceable to the
/// identity that produced it. The name is bounded, and only the characters
/// Qdrant accepts for a collection name are kept.
fn fresh_target_name(base_collection: &str, fingerprint: &str) -> String {
    let base: String = base_collection
        .trim()
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '_' || *character == '-'
        })
        .take(64)
        .collect();
    let base = if base.is_empty() {
        "context69".to_string()
    } else {
        base
    };
    let identity: String = fingerprint.chars().take(16).collect();
    format!("{base}__{identity}__{}", Uuid::new_v4().simple())
}

/// The identity and count an admitted rebuild writes once it succeeds.
pub(crate) struct RebuiltIdentity {
    pub(crate) collection: String,
    pub(crate) fingerprint: String,
    pub(crate) qdrant_url: String,
    pub(crate) base_collection: String,
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
            qdrant_url: self.qdrant.url.clone(),
            base_collection: self.base_collection.clone(),
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

/// The refusal a rebuild start reports to its caller.
fn rebuild_refusal_error(reason: crate::db::RebuildRefusalReason) -> anyhow::Error {
    match reason {
        crate::db::RebuildRefusalReason::StagingEntries => DomainError::conflict(
            "an unresolved ingestion run still holds staged points; retry the rebuild after it \
             resolved",
        )
        .into(),
        crate::db::RebuildRefusalReason::LegacyStaging => DomainError::conflict(
            "the rebuild started before document staging existed and cannot be resumed; cancel \
             it and start a new rebuild",
        )
        .into(),
        crate::db::RebuildRefusalReason::Busy => RebuildBusy.into(),
        crate::db::RebuildRefusalReason::IdentityMismatch => DomainError::conflict(
            "a vector index rebuild of another vector identity is pending; an explicit rebuild of \
             the current identity is required",
        )
        .into(),
        crate::db::RebuildRefusalReason::PersistedIdentityMismatch => DomainError::conflict(
            "the published vector pair does not match the persisted vector identity; retry the \
             rebuild after the runtime applied the configured identity",
        )
        .into(),
        crate::db::RebuildRefusalReason::PersistedIdentityMissing => DomainError::unavailable(
            "the persisted vector identity is unavailable; the rebuild cannot know which vector \
             space it would rebuild",
        )
        .into(),
    }
}

/// Commits the identity of a finished rebuild, clears its barrier and moves
/// the epochs in one transaction.
///
/// The transaction runs under the captured grant of the task item, so the
/// completion is authorized by the same owner the barrier records. A barrier
/// this attempt no longer owns writes nothing at all: the identity of the
/// rebuild is not persisted either, so a superseded attempt never describes a
/// collection another owner rebuilt.
async fn complete_rebuild_transaction(
    db: &crate::db::Database,
    fence: TaskWriteFence,
    execution: &crate::db::ExecutionContext,
    task_id: Uuid,
    attempt_id: i64,
    identity: &RebuiltIdentity,
) -> Result<()> {
    // The completion opens a plain transaction: unlike an ordinary task write
    // it deliberately makes the item terminal, so it must not run the generic
    // TaskWriteTx commit whose grant recheck expects a still-running item.
    let mut tx = db.pool().begin().await?;
    // The canonical order is item -> epoch -> vector identity rows -> task. The
    // item is locked and its captured grant verified before the epoch is taken,
    // exactly like every other worker and cancellation path.
    let locked = sqlx::query_file_as!(
        LockedCompletionItem,
        "src/sql/db/tasks/lock_task_item.sql",
        fence.item_id(),
        fence.token(),
        fence.execution_task_id(),
        fence.generation(),
        None::<chrono::DateTime<chrono::Utc>>
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(locked) = locked else {
        tx.rollback().await.ok();
        return Err(anyhow::anyhow!(
            "the rebuild completion item is no longer available"
        ));
    };
    if !locked.owned || !locked.matches_execution {
        tx.rollback().await.ok();
        return Err(anyhow::anyhow!(
            "the rebuild completion grant is no longer owned by this attempt (item status {})",
            locked.status
        ));
    }
    // The epoch is taken under that item lock, so the barrier ownership and
    // the identity rows describe one instant. A superseded owner writes
    // nothing: no identity, no active switch, no epoch.
    let cleared = db
        .complete_owned_rebuild_in(
            &mut tx,
            fence.item_id(),
            fence.token(),
            &crate::db::VectorIndexState {
                collection_name: &identity.collection,
                fingerprint: &identity.fingerprint,
                qdrant_url: &identity.qdrant_url,
                base_collection: &identity.base_collection,
                embedding_base_url: &identity.base_url,
                embedding_model: &identity.model,
                dimensions: identity.dimensions,
                rebuilt_chunks: identity.rebuilt_chunks,
            },
        )
        .await?;
    if cleared.is_none() {
        tx.rollback().await.ok();
        return Err(anyhow::anyhow!(
            "the rebuild barrier is no longer owned by this attempt"
        ));
    }
    // The captured library segment and attempt are sealed in the same
    // transaction as the activation, before the domain transition. A crash
    // after the commit therefore leaves every row family terminal: the domain
    // item and parent succeed, and the segment still reports the same success
    // instead of waiting for a worker that may never run again.
    let queue = db.execution_queue().await?;
    queue.complete_in(&mut tx, &execution.attempt, None).await?;
    // Activation is the completion linearization point: the owner item is
    // finished in the same owner-guarded transaction. A cancellation that
    // waited for the item fence then sees the completed item and reports too
    // late instead of mutating the activated rebuild into a cancelled one.
    // The captured library attempt is sealed by the worker's normal success
    // path, which observes the already-finished item and projects the parent.
    let resource_id = task_id.to_string();
    let finished = sqlx::query_file_scalar!(
        "src/sql/db/tasks/finish_item.sql",
        fence.item_id(),
        "succeeded",
        Some(resource_id.as_str()),
        Option::<&str>::None,
        Option::<&str>::None,
        false,
        fence.token(),
        attempt_id,
        fence.execution_task_id(),
        fence.generation()
    )
    .fetch_one(&mut *tx)
    .await?;
    if !finished {
        tx.rollback().await.ok();
        return Err(anyhow::anyhow!(
            "the rebuild activation could not finish its task item"
        ));
    }
    // The parent projection commits with the activation, so a process that
    // dies immediately after the commit still observes a completed task rather
    // than a running task whose only item already succeeded.
    sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
        .execute(&mut *tx)
        .await?;
    // The grant was verified under the item lock before the terminal
    // transition, and finish_item's own predicate is the final fence. Committing
    // the plain transaction is the activation linearization point.
    tx.commit().await?;
    Ok(())
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
    // The verified target is published at the admitted generation before the
    // rebuild fence is cleared, so the runtime never serves the old pair as
    // the live one after the activation committed. A superseded generation
    // publishes nothing and leaves the runtime closed.
    if !vector.publish_admitted_target(attempt.generation, attempt.snapshot.clone()) {
        return discard_superseded_rebuild(tracker, attempt).await;
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
        RebuildAttempt, RebuildStartReservation, RebuiltIdentity, VectorRebuildTracker,
        discard_superseded_rebuild, fail_rebuild_attempt, fresh_target_name, install_completion,
        original_reservation_run_id, settle_successful_rebuild,
    };
    use crate::{
        config::QdrantConfig,
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

    fn snapshot(
        generation: u64,
        collection: &str,
        model: &str,
        fingerprint: &str,
    ) -> VectorSnapshot {
        VectorSnapshot {
            embedding: Arc::new(TestProvider),
            identity: Some(identity(model)),
            index: Arc::new(QdrantIndex::for_test(collection, 8)),
            collection: collection.to_string(),
            generation,
            configured_fingerprint: Some(fingerprint.to_string()),
        }
    }

    /// A runtime with a genuinely published, fingerprinted pair at its current
    /// generation, exactly as recovery publishes one.
    fn runtime() -> VectorRuntime {
        let runtime =
            VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let generation = runtime.generation();
        assert!(runtime.publish(
            generation,
            snapshot(generation, "collection-a", "model-a", "fingerprint-a")
        ));
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

    /// Admits an attempt through the production target admission API, not a
    /// fabricated matching snapshot.
    fn admit(runtime: &VectorRuntime) -> RebuildAttempt {
        let generation = runtime.generation();
        let admitted = runtime
            .try_admit_target_rebuild(
                snapshot(generation, "collection-a", "model-a", "fingerprint-a"),
                "fingerprint-a".to_string(),
            )
            .expect("the current generation admits a target rebuild");
        RebuildAttempt {
            generation: admitted.generation,
            snapshot: admitted.snapshot,
            fingerprint: admitted.configured_fingerprint,
            qdrant: QdrantConfig {
                url: "http://127.0.0.1:1".to_string(),
                collection_name: "collection-a".to_string(),
                recreate_on_dimension_mismatch: false,
            },
            base_collection: "collection-a".to_string(),
            dimensions: 8,
            chunk_budget: crate::chunking::ChunkingRuntime::new(
                crate::chunking::ChunkBudget::default(),
            )
            .resolve_snapshot()
            .expect("the default chunk budget resolves"),
            rebuild_generation: 1,
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

    #[test]
    fn the_original_reservation_run_id_is_deterministic_per_target_file() {
        let file = uuid::Uuid::from_u128(1);
        let without_file = original_reservation_run_id("target-a", None);
        assert_eq!(
            without_file,
            original_reservation_run_id("target-a", None),
            "the same target and file derive one reservation id"
        );
        let with_file = original_reservation_run_id("target-a", Some(file));
        assert_ne!(without_file, with_file);
        assert_ne!(
            with_file,
            original_reservation_run_id("target-b", Some(file))
        );
        assert_ne!(
            with_file,
            original_reservation_run_id("target-a", Some(uuid::Uuid::from_u128(2)))
        );
    }

    #[test]
    fn the_installation_completes_only_on_an_exhausted_matching_source() {
        assert!(
            install_completion(0, 0, true).expect("a zero-total build settles"),
            "an empty source with a zero total is a complete installation"
        );
        assert!(
            install_completion(7, 7, true).expect("a matching source completes"),
            "an exhausted source with the pinned total completes"
        );
        assert!(
            !install_completion(3, 7, false).expect("a page in flight is not complete"),
            "a source that still has rows never completes"
        );
        assert!(
            install_completion(3, 7, true).is_err(),
            "a source that ended short must abort instead of publishing"
        );
    }

    #[test]
    fn a_reclaimed_paused_owner_parks_instead_of_retrying_qdrant() {
        let item = uuid::Uuid::new_v4();
        let mut barrier = crate::db::RebuildBarrier {
            state: crate::db::RebuildBarrierState::Running,
            owner_item_id: Some(item),
            owner_token: Some(uuid::Uuid::new_v4()),
            fingerprint: Some("fingerprint-a".to_string()),
            collection: Some("target".to_string()),
            started_at: None,
            error: None,
            pause_requested: true,
            cursor: None,
            processed_chunks: 2,
            total_chunks: 8,
            rebuild_target_created: false,
            document_index_invalid: false,
            install_started: false,
            install_cursor: None,
            installed_chunks: 0,
            install_total_chunks: 0,
        };
        assert!(
            super::paused_rebuild_owned_by(&barrier, item),
            "the reclaimed item of the paused owner reaches the paused dependency"
        );
        assert!(
            !super::paused_rebuild_owned_by(&barrier, uuid::Uuid::new_v4()),
            "another item never parks on this paused owner"
        );

        barrier.pause_requested = false;
        assert!(
            !super::paused_rebuild_owned_by(&barrier, item),
            "a resumed (unpaused) barrier starts normally"
        );
        barrier.pause_requested = true;
        barrier.state = crate::db::RebuildBarrierState::Failed;
        assert!(
            !super::paused_rebuild_owned_by(&barrier, item),
            "a failed barrier is not a parked pause"
        );
    }

    #[test]
    fn a_fresh_target_name_is_unique_bounded_and_carries_its_identity() {
        let first = fresh_target_name("context69", "abcdef0123456789abcdef");
        let second = fresh_target_name("context69", "abcdef0123456789abcdef");
        assert_ne!(
            first, second,
            "two builds never share a physical collection"
        );
        assert_ne!(
            first, "context69",
            "a fresh target never reuses the logical base name a retired collection may hold"
        );
        assert!(
            first.starts_with("context69__abcdef0123456789__"),
            "{first}"
        );
        assert!(
            first.len() <= 64 + 2 + 16 + 2 + 32,
            "the target name is bounded: {first}"
        );

        // A name that carries characters Qdrant does not accept keeps only the
        // ones it does.
        let sanitized = fresh_target_name("context 69/vectors!", "0123456789abcdef");
        assert!(!sanitized.contains(' '), "{sanitized}");
        assert!(!sanitized.contains('/'), "{sanitized}");
        assert!(!sanitized.contains('!'), "{sanitized}");
        assert!(sanitized.starts_with("context69vectors__"), "{sanitized}");
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
        assert_eq!(identity.qdrant_url, "http://127.0.0.1:1");
        assert_eq!(identity.base_collection, "collection-a");
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
        assert!(runtime.publish(
            newer,
            snapshot(newer, "collection-b", "model-b", "fingerprint-b")
        ));
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
            snapshot(new_generation, "collection-b", "model-b", "fingerprint-b")
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

    #[test]
    fn a_dropped_start_reservation_releases_the_gate_for_the_next_attempt() {
        let tracker = VectorRebuildTracker::new();
        assert!(tracker.claim_start(), "the first start reserves the slot");
        {
            let _reservation = RebuildStartReservation {
                tracker: &tracker,
                armed: true,
            };
            assert!(
                !tracker.claim_start(),
                "an armed reservation holds the start slot"
            );
        }
        assert!(
            tracker.claim_start(),
            "a dropped reservation releases the slot for the next attempt"
        );
        tracker.release_start();
    }

    #[test]
    fn a_settled_start_reservation_lets_a_finished_build_start_again() {
        let tracker = VectorRebuildTracker::new();
        assert!(tracker.claim_start());
        let reservation = RebuildStartReservation {
            tracker: &tracker,
            armed: true,
        };
        tracker.begin(1);
        reservation.settle();
        assert!(tracker.succeed(1, 0));
        assert!(
            tracker.claim_start(),
            "a settled reservation releases the gate after the running build finishes"
        );
        tracker.release_start();
    }

    #[test]
    fn a_settled_start_reservation_lets_a_paused_build_start_again() {
        let tracker = VectorRebuildTracker::new();
        assert!(tracker.claim_start());
        let reservation = RebuildStartReservation {
            tracker: &tracker,
            armed: true,
        };
        tracker.begin(1);
        reservation.settle();
        assert!(tracker.fail(1, "paused".to_string()));
        assert!(
            tracker.claim_start(),
            "a paused build leaves no start gate behind; only the durable barrier decides resume"
        );
        tracker.release_start();
    }
}
