use super::*;
use std::time::Instant;

impl SyncService {
    pub async fn sync_all(&self, trigger: &str) -> Result<()> {
        let source_keys = self.registry.read().await.source_keys();
        if source_keys.is_empty() {
            return Ok(());
        }
        let runtime = self.runtime()?.clone();
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
            Err(anyhow!(errors.join("; ")))
        }
    }

    pub async fn sync_source(&self, source_key: &str, trigger: &str) -> Result<SyncOutcome> {
        let started = Instant::now();
        let _guard = self.acquire_lock(source_key).await?;
        let (source, connector) = self
            .source_runtime(source_key)
            .await?
            .with_context(|| format!("unknown source {source_key}"))?;
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
        let runtime = self.runtime()?.clone();
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
            .with_context(|| format!("missing source scope for {}", source.key))?;
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

                let seed_payload = ChunkPayload {
                    chunk_id: uuid::Uuid::nil(),
                    document_id: 0,
                    group_id: source_scope.group_id,
                    group_key: source_scope.group_key.clone(),
                    group_path: source_scope.group_path.clone(),
                    visibility: source_scope.visibility,
                    source_key: source.key.clone(),
                    external_id: normalized.external_id.clone(),
                    title: normalized.title.clone(),
                    summary: normalized.summary.clone(),
                    source_uri: normalized.source_uri.clone(),
                    published_at: normalized.published_at,
                    updated_at_source: normalized.updated_at,
                    record_hash: normalized.record_hash.clone(),
                    chunk_index: 0,
                    chunk_text: normalized.body_text.clone(),
                    metadata_json: normalized.metadata_json.clone(),
                    content_locale: "original".to_string(),
                    source_locale: None,
                    translation_provider: None,
                };
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
                    let embeddings = runtime.embedding.embed_texts(&texts).await?;
                    let payloads = chunks
                        .iter()
                        .map(|chunk| ChunkPayload {
                            chunk_id: chunk.id,
                            document_id: upserted.document_id,
                            group_id: source_scope.group_id,
                            group_key: source_scope.group_key.clone(),
                            group_path: source_scope.group_path.clone(),
                            visibility: source_scope.visibility,
                            source_key: source.key.clone(),
                            external_id: normalized.external_id.clone(),
                            title: normalized.title.clone(),
                            summary: normalized.summary.clone(),
                            source_uri: normalized.source_uri.clone(),
                            published_at: normalized.published_at,
                            updated_at_source: normalized.updated_at,
                            record_hash: normalized.record_hash.clone(),
                            chunk_index: chunk.chunk_index,
                            chunk_text: chunk.text.clone(),
                            metadata_json: normalized.metadata_json.clone(),
                            content_locale: "original".to_string(),
                            source_locale: None,
                            translation_provider: None,
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

    pub async fn rebuild_index_from_db(&self) -> Result<usize> {
        let runtime = self.runtime()?.clone();
        let total_chunks = self.db.count_chunk_payloads_for_reindex().await?;
        self.set_vector_rebuild_progress(0, total_chunks).await;
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
            let embeddings = runtime.embedding.embed_texts(&texts).await?;
            runtime
                .index
                .replace_document_chunks(&[], &batch, &embeddings)
                .await?;
            rebuilt += batch.len();
            let last = batch.last().expect("non-empty reindex batch");
            last_document_id = Some(last.document_id);
            last_chunk_index = Some(last.chunk_index);
            self.set_vector_rebuild_progress(rebuilt, total_chunks)
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
        self.vector_rebuild_status.read().await.clone()
    }

    pub async fn start_vector_index_rebuild(&self) -> Result<VectorIndexRebuildStatus> {
        self.runtime()?;
        self.begin_vector_index_rebuild().await?;

        let service = self.clone();
        tokio::spawn(async move {
            let result = service.rebuild_index_from_db().await;
            service.finish_vector_index_rebuild(result).await;
        });

        Ok(self.vector_index_rebuild_status().await)
    }

    pub(crate) async fn begin_vector_index_rebuild(&self) -> Result<()> {
        self.runtime()?;
        let mut status = self.vector_rebuild_status.write().await;
        if status.state == VectorIndexRebuildState::Running {
            return Err(anyhow!("vector index rebuild is already running"));
        }
        *status = VectorIndexRebuildStatus {
            state: VectorIndexRebuildState::Running,
            processed_chunks: 0,
            total_chunks: 0,
            error_message: None,
            started_at: Some(chrono::Utc::now()),
            finished_at: None,
        };
        Ok(())
    }

    pub(crate) async fn finish_vector_index_rebuild(&self, result: Result<usize>) {
        let mut status = self.vector_rebuild_status.write().await;
        status.finished_at = Some(chrono::Utc::now());
        match result {
            Ok(rebuilt) => {
                status.state = VectorIndexRebuildState::Succeeded;
                status.processed_chunks = rebuilt;
                status.total_chunks = rebuilt;
            }
            Err(error) => {
                error!(error = %error, "vector index rebuild failed");
                status.state = VectorIndexRebuildState::Failed;
                status.error_message = Some(error.to_string());
            }
        }
    }

    async fn set_vector_rebuild_progress(&self, processed_chunks: usize, total_chunks: usize) {
        let mut status = self.vector_rebuild_status.write().await;
        if status.state == VectorIndexRebuildState::Running {
            status.processed_chunks = processed_chunks;
            status.total_chunks = total_chunks;
        }
    }
}
