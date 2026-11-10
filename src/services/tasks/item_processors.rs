use crate::db::Database;
use crate::domain_errors::DomainError;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use context69_contracts::TaskKind;
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use super::pipeline_progress::payload_after_sections_saved;
use crate::services::library::UnifiedIngestError;

/// Outcome of one item run under the collapsed stage machine (issue 529
/// Task 4). `Progressed` never leaves the worker: the blocking driver in
/// [`process_item_blocking`] consumes it and continues with `next` inside the
/// same claim, so no stage is persisted and no item is re-claimed between
/// steps.
pub(super) enum ProcessResult {
    Succeeded(Option<String>),
    Progressed {
        next: &'static str,
    },
    Waiting {
        reason: String,
        dependency_key: Option<String>,
        next_attempt_at: DateTime<Utc>,
        message: Option<String>,
    },
    Failed {
        stage: String,
        message: String,
        retryable: bool,
    },
    /// The parent task requested a cooperative pause and the current attempt
    /// reached a safe stage boundary.
    Paused,
}

/// Upper bound for the steps one blocking run may take. Every step either
/// finishes the item, parks it on a wait/failure, or advances; a pipeline that
/// keeps advancing is a bug, so the driver fails the item instead of spinning.
const MAX_ITEM_STAGES: usize = 16;

/// Run one claimed item to completion.
///
/// The whole pipeline runs inline: download/storage/docling/embedding/indexing/
/// translation exist only as function calls here, never as queue
/// values. A step that cannot finish parks the item (`Waiting`/`Failed`) and
/// the next claim restarts from the kind's entry stage, where every step
/// re-derives its progress from the persisted payload and file state.
///
/// Stages persist `file_id`/`payload` as they run and write the same values
/// back into one shared snapshot, so a later stage in the same claim observes
/// what an earlier stage saved instead of the stale claim (issue 592). The
/// working snapshot is written back into `item` on return so an inline retry
/// re-drive (issue 650 P3) resumes from fresh progress instead of redoing
/// persisted stages.
pub(super) async fn process_item_blocking(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
) -> Result<ProcessResult> {
    let mut working = item.clone();
    pin_chunk_budget(service, &mut working).await?;
    let budget = crate::chunking::parse_chunk_budget(&working.payload)?
        .ok_or_else(|| DomainError::internal("task item chunk budget was not pinned"))?;
    let service = &service.scoped_chunking(&budget);
    if task.origin == "rerun_source" {
        let outcome = super::item_replacement_processor::process_replacement(
            service,
            task,
            group,
            &mut working,
        )
        .await?;
        *item = working;
        return Ok(outcome);
    }
    let stage = resume_stage(kind, working.stage.as_deref());
    let runner = ServiceStageRunner {
        service,
        kind,
        group,
        task,
    };
    let outcome = drive_item(&mut working, stage, &runner).await?;
    *item = working;
    Ok(outcome)
}

/// One pipeline step. The runner reads and updates the shared item snapshot,
/// so a later stage observes what an earlier stage persisted.
#[async_trait::async_trait]
pub(super) trait ItemStageRunner: Send + Sync {
    async fn run(
        &self,
        item: &mut crate::db::ClaimedItem,
        stage: &'static str,
    ) -> Result<ProcessResult>;
}

/// Advance one mutable item snapshot through the collapsed pipeline.
pub(super) async fn drive_item(
    item: &mut crate::db::ClaimedItem,
    mut stage: &'static str,
    runner: &(dyn ItemStageRunner + '_),
) -> Result<ProcessResult> {
    for _ in 0..MAX_ITEM_STAGES {
        // Entry into the stage the worker is about to run. The caller's
        // lifecycle span already carries task/item/attempt/ordinal, so this
        // event only has to name the stage itself. Debug: a stage advance is
        // not a queue-visible transition (the column is not persisted
        // mid-run) and fires up to `MAX_ITEM_STAGES` times per item.
        tracing::debug!(target: "task_lifecycle", entered_stage = stage, "task item entered stage");
        match runner.run(item, stage).await? {
            ProcessResult::Progressed { next } => stage = next,
            terminal => return Ok(terminal),
        }
    }
    Ok(ProcessResult::Failed {
        stage: stage.to_string(),
        message: format!("item pipeline did not converge within {MAX_ITEM_STAGES} steps"),
        retryable: false,
    })
}

struct ServiceStageRunner<'a> {
    service: &'a TaskService,
    kind: TaskKind,
    group: Option<&'a crate::domain::GroupRecord>,
    task: &'a crate::db::StoredTask,
}

#[async_trait::async_trait]
impl ItemStageRunner for ServiceStageRunner<'_> {
    async fn run(
        &self,
        item: &mut crate::db::ClaimedItem,
        stage: &'static str,
    ) -> Result<ProcessResult> {
        if self.service.pause_requested(item).await? {
            return Ok(ProcessResult::Paused);
        }
        run_stage(self.service, self.kind, self.group, self.task, item, stage).await
    }
}

/// Stage a claim starts at. `processing` is the collapsed default written at
/// creation; a pre-collapse stage value also restarts at the kind's entry
/// stage because the steps are resumable. `finalize` is the one terminal
/// marker: the work is already done and only completion bookkeeping is left.
fn resume_stage(kind: TaskKind, stage: Option<&str>) -> &'static str {
    match stage {
        Some("finalize") => "finalize",
        _ => entry_stage(kind),
    }
}

/// The stage this claim enters, for the lifecycle log.
///
/// The persisted `stage` column is the collapsed `processing` marker and never
/// advances mid-run, so an attempt log that reports it names a stage no
/// operator can act on. This is the real entry stage the worker starts at.
pub(super) fn resolve_entry_stage(kind: TaskKind, stage: Option<&str>) -> &'static str {
    resume_stage(kind, stage)
}

fn entry_stage(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::UrlBatch => "download",
        TaskKind::TextBatch | TaskKind::FileBatch => "storage",
        TaskKind::DeleteBatch => "delete",
        TaskKind::SourceSync => "sync",
        TaskKind::VectorRebuild => "indexing",
        TaskKind::Translation => "translation",
        TaskKind::GitIndex => "indexing",
    }
}

async fn run_stage(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    match kind {
        TaskKind::TextBatch => {
            super::item_file_processors::process_text(service, group, task, item, stage).await
        }
        TaskKind::FileBatch => {
            super::item_file_processors::process_file(service, group, task, item, stage).await
        }
        TaskKind::UrlBatch => {
            super::item_url_processor::process_url(service, group, task, item, stage).await
        }
        TaskKind::DeleteBatch => {
            super::item_lifecycle_processors::process_delete(service, group, item, stage).await
        }
        TaskKind::SourceSync => {
            super::item_lifecycle_processors::process_sync(service, group, task, item, stage).await
        }
        TaskKind::VectorRebuild => {
            super::item_lifecycle_processors::process_vector_rebuild(service, task, item, stage)
                .await
        }
        TaskKind::Translation => {
            super::item_translation_processors::process_translation(
                service, group, task, item, stage,
            )
            .await
        }
        TaskKind::GitIndex => {
            super::item_git_index_processor::process_git_index(service, group, item, stage).await
        }
    }
}

/// Pins the chunk budget of an item that predates durable snapshots.
///
/// A task submitted after P3 already carries its snapshot. A legacy item has
/// no recoverable original configuration, so it is pinned once, before any
/// stage runs, to the legacy character budget. The write is lease-fenced, so a
/// worker that lost its claim leaves the payload untouched and the next claim
/// pins it instead. Pinning is durable on purpose: a retry must resume under
/// the same budget, not a transient one.
async fn pin_chunk_budget(service: &TaskService, item: &mut crate::db::ClaimedItem) -> Result<()> {
    if crate::chunking::parse_chunk_budget(&item.payload)?.is_some() {
        return Ok(());
    }
    let budget = service.chunking().legacy_chars_budget();
    let payload = crate::chunking::payload_with_chunk_budget(&item.payload, &budget);
    let fence = crate::library_store::TaskWriteFence::from_claimed_item(item)?;
    if !crate::library_store::write_task_item_payload(service.db(), fence, &payload).await? {
        return Err(DomainError::conflict(
            "task item lease was lost while pinning the chunking budget",
        )
        .into());
    }
    item.payload = payload;
    Ok(())
}

pub(super) async fn set_file(
    db: &Database,
    task_id: Uuid,
    item: &mut crate::db::ClaimedItem,
    file_id: Uuid,
) -> Result<()> {
    // The captured execution is the authority of the write: without it the
    // item was never claimed through the library.
    let execution = item.execution_context().ok_or_else(|| {
        anyhow::Error::from(DomainError::conflict("task item has no captured execution"))
    })?;
    if !db
        .set_task_item_file(task_id, item.id, &execution, file_id)
        .await?
    {
        return Err(DomainError::conflict("task item lease was lost while saving file_id").into());
    }
    item.file_id = Some(file_id);
    Ok(())
}

pub(super) async fn save_sections(
    service: &TaskService,
    item: &mut crate::db::ClaimedItem,
    sections: Value,
) -> Result<()> {
    let mut payload = item.payload.clone();
    payload["section_payload"] = sections.clone();
    // New sections invalidate any previous batch progress; start at 0.
    // Preserve boundedness: remove small checkpoint rather than carrying
    // stale hash/total. The next indexing stage will recreate it with the
    // current hash.
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("indexing_checkpoint");
    }
    let payload = payload_after_sections_saved(item, &payload)?;
    let fence = crate::library_store::TaskWriteFence::from_claimed_item(item)?;
    if !crate::library_store::write_task_item_payload(service.db(), fence, &payload).await? {
        return Err(DomainError::conflict("task item lease was lost while saving sections").into());
    }
    item.payload = payload;
    // The inline save is durable first; externalization then removes only that
    // inline copy once a verified artifact reference commits. A lease loss or
    // storage error leaves the durable inline checkpoint in place and fails the
    // item, so the next claim re-drives from valid state.
    service
        .library()
        .externalize_task_item_sections(item, &sections)
        .await?;
    Ok(())
}

/// Resolve the sections a task item pipeline stage must consume.
///
/// Inline legacy bytes win, so existing rows keep their exact behavior.
/// Otherwise a verified `sections` artifact is read and decoded into the same
/// JSON shape. Unlike the pure [`persisted_section_payload`], a present but
/// unreadable, unverifiable or undecodable artifact is an error, so a caller
/// never mistakes it for "no sections" and reconverts.
pub(super) async fn resolved_section_payload(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
) -> Result<Option<Value>> {
    if let Some(inline) = persisted_section_payload(&item.payload) {
        return Ok(Some(inline));
    }
    if item.group_id.is_none() {
        return Ok(None);
    }
    let Some(bytes) = service
        .library()
        .read_task_item_artifact(
            item.id,
            crate::db::TaskItemArtifactKind::Sections,
            crate::services::library::MAX_TASK_ITEM_ARTIFACT_BYTES,
        )
        .await?
    else {
        return Ok(None);
    };
    let sections = serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::Error::from(DomainError::conflict(format!(
            "task item sections artifact is not valid JSON: {error}"
        )))
    })?;
    Ok(Some(sections))
}

pub(super) async fn save_payload(
    db: &Database,
    item: &mut crate::db::ClaimedItem,
    payload: Value,
) -> Result<()> {
    let fence = crate::library_store::TaskWriteFence::from_claimed_item(item)?;
    if !crate::library_store::write_task_item_payload(db, fence, &payload).await? {
        return Err(DomainError::conflict("task item lease was lost while saving payload").into());
    }
    item.payload = payload;
    Ok(())
}

pub(crate) fn persisted_section_payload(payload: &Value) -> Option<Value> {
    payload
        .get("section_payload")
        .filter(|value| !value.is_null())
        .cloned()
}

pub(super) fn waiting_for_error(
    item: &crate::db::ClaimedItem,
    error: UnifiedIngestError,
) -> ProcessResult {
    let attempt = item.attempt_count.clamp(1, 8) as u32;
    let seconds = 5_i64.saturating_mul(1_i64 << (attempt - 1));
    ProcessResult::Waiting {
        reason: if error.dependency_key.is_some() {
            "dependency".to_string()
        } else {
            "backoff".to_string()
        },
        dependency_key: error.dependency_key,
        next_attempt_at: Utc::now() + ChronoDuration::seconds(seconds.min(300)),
        message: Some(error.message),
    }
}

/// The captured-ownership refusal: the item was never claimed through the
/// library, so neither the processors nor the write fence can build an
/// authority for it.
///
/// The typed conflict carries this phrase and the write fence's own refusal is
/// an untyped error with the same code-owned wording, so the retry decision
/// recognises both without decoding a provider message.
const MISSING_CAPTURED_EXECUTION: &str = "has no captured execution";

/// Whether one escaped task failure may be retried automatically.
///
/// The typed retry policy decides, except that the captured-ownership refusal is
/// terminal: no later attempt can give the item the execution authority the
/// fence requires, so retrying only repeats the same refusal.
pub(super) fn is_retryable_failure(error: &anyhow::Error) -> bool {
    !captured_execution_is_missing(error) && crate::retry_policy::is_transient(error)
}

fn captured_execution_is_missing(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<DomainError>(),
            Some(DomainError::Conflict(message)) if message.contains(MISSING_CAPTURED_EXECUTION)
        ) || cause.to_string().contains(MISSING_CAPTURED_EXECUTION)
    })
}

pub(super) fn process_error(stage: &str, error: anyhow::Error) -> ProcessResult {
    let message = error.to_string();
    ProcessResult::Failed {
        stage: stage.to_string(),
        retryable: is_retryable_failure(&error),
        message,
    }
}

pub(super) fn process_source_sync_error(
    item: &crate::db::ClaimedItem,
    error: anyhow::Error,
) -> ProcessResult {
    // A source sync that cannot serve vector work waits on the dependency with
    // the existing backoff instead of consuming a normal failure attempt.
    if let Some(unavailable) = UnifiedIngestError::vector_runtime_unavailable("sync", &error) {
        return waiting_for_error(item, unavailable);
    }
    if let Some(ingest_error) = error.downcast_ref::<UnifiedIngestError>() {
        if ingest_error.retryable {
            return waiting_for_error(item, ingest_error.clone());
        }
        return ProcessResult::Failed {
            stage: ingest_error.stage.clone(),
            message: ingest_error.message.clone(),
            retryable: false,
        };
    }
    process_error("sync", error)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_ITEM_STAGES, ProcessResult, entry_stage, is_retryable_failure, process_error,
        process_source_sync_error, resolve_entry_stage, resume_stage,
    };
    use context69_contracts::TaskKind;

    fn claimed_item(attempt_count: i32) -> crate::db::ClaimedItem {
        crate::db::ClaimedItem {
            id: uuid::Uuid::nil(),
            task_id: uuid::Uuid::nil(),
            ordinal: 0,
            attempt_count,
            lease_token: uuid::Uuid::nil(),
            attempt_id: 0,
            payload: serde_json::json!({}),
            file_id: None,
            stage: None,
            input_storage_object_id: None,
            kind: "source_sync".to_string(),
            group_id: None,
            group_path: None,
            source_key: None,
            // The fake was never claimed through the library, so it carries no
            // captured execution and may not write through a fenced method.
            execution: None,
            execution_task_id: None,
            execution_generation: 0,
        }
    }

    #[test]
    fn a_vector_runtime_wait_parks_the_item_without_burning_the_attempt() {
        let item = claimed_item(2);
        let error = anyhow::anyhow!(
            "vector index is rebuilding or unavailable; retry after the rebuild completes"
        );

        let result = process_source_sync_error(&item, error);
        match result {
            ProcessResult::Waiting {
                reason,
                dependency_key,
                next_attempt_at,
                message,
            } => {
                assert_eq!(reason, "dependency");
                assert_eq!(dependency_key.as_deref(), Some("qdrant"));
                assert!(next_attempt_at > chrono::Utc::now());
                assert!(
                    message
                        .as_deref()
                        .is_some_and(|message| message.contains("rebuilding")),
                    "the wait must carry the runtime's own explanation"
                );
            }
            _ => panic!("expected a dependency wait"),
        }
        // The conversion only reads the claim: no counter, status, or stage
        // mutation happens at this boundary.
        assert_eq!(item.attempt_count, 2);
        assert_eq!(item.stage, None);
    }

    #[test]
    fn an_ordinary_source_sync_failure_keeps_the_failure_path() {
        let item = claimed_item(1);
        let error = anyhow::anyhow!("source returns an unsupported schema");

        match process_source_sync_error(&item, error) {
            // The message no longer decides: an untyped failure stays transient
            // and keeps the existing backoff, whatever words it carries.
            ProcessResult::Failed { retryable, .. } => assert!(retryable),
            _ => panic!("expected a failure"),
        }
    }

    /// A deterministic failure stops the item on its first attempt: the stage
    /// is recorded, the message is the actionable one, and no automatic retry
    /// is scheduled.
    #[test]
    fn a_deterministic_failure_stops_the_item_without_a_retry() {
        let error: anyhow::Error = crate::domain_errors::DomainError::payload_too_large(
            "remote_file_too_large: limit_bytes=1024 observed_bytes=4096 source=content_length",
        )
        .into();

        match process_error("download", error) {
            ProcessResult::Failed {
                stage,
                message,
                retryable,
            } => {
                assert_eq!(stage, "download");
                assert!(!retryable, "a size failure must not be retried");
                assert!(message.contains("limit_bytes=1024"));
                assert!(message.contains("observed_bytes=4096"));
            }
            _ => panic!("expected a failure"),
        }
    }

    /// A transient failure keeps the automatic retry.
    #[test]
    fn a_transient_failure_keeps_the_automatic_retry() {
        let error: anyhow::Error = anyhow::anyhow!("database connection reset");

        match process_error("storage", error) {
            ProcessResult::Failed { retryable, .. } => assert!(retryable),
            _ => panic!("expected a failure"),
        }
    }

    #[test]
    fn collapsed_pipeline_starts_at_the_kind_entry_stage() {
        assert_eq!(entry_stage(TaskKind::UrlBatch), "download");
        assert_eq!(entry_stage(TaskKind::TextBatch), "storage");
        assert_eq!(entry_stage(TaskKind::FileBatch), "storage");
        assert_eq!(entry_stage(TaskKind::DeleteBatch), "delete");
        assert_eq!(entry_stage(TaskKind::SourceSync), "sync");
        assert_eq!(entry_stage(TaskKind::VectorRebuild), "indexing");
        assert_eq!(entry_stage(TaskKind::Translation), "translation");
        assert_eq!(entry_stage(TaskKind::GitIndex), "indexing");
    }

    #[test]
    fn processing_and_legacy_stages_restart_but_finalize_is_terminal() {
        // The collapsed default and every pre-collapse stage value restart from
        // the entry stage: each step re-derives its progress from payload and
        // file state, so resuming `indexing` never re-runs the pipeline from a
        // persisted stage machine.
        for stage in [Some("processing"), Some("indexing"), None] {
            assert_eq!(resume_stage(TaskKind::FileBatch, stage), "storage");
        }
        assert_eq!(
            resume_stage(TaskKind::UrlBatch, Some("storage")),
            "download"
        );
        // `finalize` is the terminal marker written by the success path.
        assert_eq!(
            resume_stage(TaskKind::FileBatch, Some("finalize")),
            "finalize"
        );
    }

    #[test]
    fn resolve_entry_stage_reports_the_stage_a_claim_actually_enters() {
        // Issue 702 P3 review: the attempt-start log reported the persisted
        // `processing` marker, which never advances mid-run, so it named a
        // stage no operator could act on. The lifecycle log must report the
        // real entry stage instead.
        for (kind, expected) in [
            (TaskKind::UrlBatch, "download"),
            (TaskKind::TextBatch, "storage"),
            (TaskKind::FileBatch, "storage"),
            (TaskKind::DeleteBatch, "delete"),
            (TaskKind::SourceSync, "sync"),
            (TaskKind::VectorRebuild, "indexing"),
            (TaskKind::Translation, "translation"),
            (TaskKind::GitIndex, "indexing"),
        ] {
            assert_eq!(
                resolve_entry_stage(kind, Some("processing")),
                expected,
                "a fresh claim of {kind:?} must report the stage it enters"
            );
        }
        assert_eq!(
            resolve_entry_stage(TaskKind::FileBatch, Some("finalize")),
            "finalize",
            "the terminal marker is the one stage that survives a re-claim"
        );
    }

    #[test]
    fn stage_budget_covers_the_longest_pipeline() {
        // URL items run the longest pipeline: download, storage, docling,
        // embedding, indexing, translation, finalize.
        let longest_pipeline = [
            "download",
            "storage",
            "docling",
            "embedding",
            "indexing",
            "translation",
            "finalize",
        ];
        assert!(MAX_ITEM_STAGES > longest_pipeline.len());
    }

    /// The captured-ownership refusal is terminal, whatever carries it.
    #[test]
    fn a_missing_captured_execution_never_keeps_the_automatic_retry() {
        let typed: anyhow::Error =
            crate::domain_errors::DomainError::conflict("task item has no captured execution")
                .into();
        assert!(!is_retryable_failure(&typed));

        let fence = anyhow::anyhow!(
            "task item {} has no captured execution and may not write",
            uuid::Uuid::nil()
        );
        assert!(!is_retryable_failure(&fence));

        // Every other conflict keeps the shared policy's transient default.
        let stale: anyhow::Error =
            crate::domain_errors::DomainError::conflict("stale target").into();
        assert!(is_retryable_failure(&stale));
    }

    /// The task-entry scope of one item.
    ///
    /// A file or URL task that reuses an already-succeeded file converts that
    /// file's translations through the library's translation service while it
    /// stores the file, before the item's own translation stage runs. The
    /// scoped clone is what keeps that conversion on the item's snapshot.
    mod chunking_scope {
        use std::path::PathBuf;
        use std::sync::Arc;

        use anyhow::Result;
        use async_trait::async_trait;
        use context69_translation::{
            TranslationChunkingBudget, TranslationDependencies, TranslationReadiness,
            TranslationService,
        };
        use uuid::Uuid;

        use crate::chunking::{ChunkBudget, ChunkingConfig, ChunkingRuntime};
        use crate::config::FileLibraryConfig;
        use crate::db::Database;
        use crate::embedding::EmbeddingRuntime;
        use crate::services::document_store::DocumentStoreService;
        use crate::services::library::{LibraryService, LibraryServiceConfig};
        use crate::services::namespace::NamespaceService;
        use crate::services::secret_store;
        use crate::services::settings::SettingsService;
        use crate::services::source_folders::SourceFoldersService;
        use crate::services::sync::SyncService;
        use crate::services::tasks::{TaskService, TaskServiceDependencies};
        use crate::services::translation::TranslationPublisherAdapter;
        use crate::vector_runtime::VectorRuntime;

        /// The translation service never becomes ready: this case observes the
        /// budget a conversion would use, it never runs a translation.
        struct NotReady;

        #[async_trait]
        impl TranslationReadiness for NotReady {
            async fn is_ready(&self) -> Result<bool> {
                Ok(false)
            }
        }

        async fn connect_db() -> Database {
            let url = context69_test_support::accessor::database_url();
            let db = Database::connect(&url)
                .await
                .expect("connect test database");
            context69_db_schema::bootstrap_library_schema(db.pool())
                .await
                .expect("bootstrap the library schema");
            db
        }

        /// A task service whose library, translation publisher, and task share
        /// one process-wide budget handle, exactly like the running
        /// application: a settings save replaces the budget in that handle.
        async fn build_task_service(db: &Database) -> (TaskService, ChunkingRuntime, PathBuf) {
            let chunking = ChunkingRuntime::new(ChunkBudget::chars(ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            }));
            let translation = TranslationService::new(TranslationDependencies {
                pool: db.pool().clone(),
                http_client: reqwest::Client::new(),
                publisher: Arc::new(TranslationPublisherAdapter::new(
                    None,
                    None,
                    chunking.clone(),
                )),
                concurrency: 1,
                readiness: Arc::new(NotReady),
            });
            let storage_root =
                std::env::temp_dir().join(format!("context69-r5-{}", Uuid::new_v4()));
            let mut library = LibraryService::with_vector_runtime(
                db.clone(),
                VectorRuntime::from_legacy(EmbeddingRuntime::new(None, None), None, ""),
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
                    embedding_vector_configured: true,
                    embedding_vector_configuration_fingerprint: "r5-chunking-scope".to_string(),
                },
                SettingsService::new(db.clone()),
                translation.clone(),
            )
            .await
            .expect("build library service");
            library.set_chunking_runtime(chunking.clone());
            let sync = SyncService::new(
                db.clone(),
                secret_store::build_unkeyed(db),
                None,
                None,
                ChunkingConfig {
                    max_chars: 1000,
                    overlap_chars: 100,
                },
                1,
                translation.clone(),
            );
            let service = TaskService::new(TaskServiceDependencies {
                db: db.clone(),
                namespace: NamespaceService::new(db.clone()),
                document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
                library: library.clone(),
                sync: sync.clone(),
                source_folders: SourceFoldersService::new(db.clone(), library, sync),
                translation,
                secrets: secret_store::build_unkeyed(db),
                concurrency: 1,
            });
            (service, chunking, storage_root)
        }

        /// A scoped item publishes under its own snapshot while the live
        /// budget a settings save installed stays untouched.
        ///
        /// The library of the scope carries the snapshot in both its chunking
        /// handle and its translation service, so a reused file converts its
        /// translations under the task's budget instead of the live one.
        #[tokio::test]
        #[ignore = "requires configured reusable PostgreSQL"]
        async fn a_task_scope_carries_its_pinned_budget_into_the_library_and_translation() {
            let db = connect_db().await;
            let (service, live, storage_root) = build_task_service(&db).await;

            // A settings save installs a new budget in the shared handle.
            let installed = ChunkBudget::chars(ChunkingConfig {
                max_chars: 2000,
                overlap_chars: 200,
            });
            live.replace(installed.clone());
            let pinned = ChunkBudget::chars(ChunkingConfig {
                max_chars: 900,
                overlap_chars: 90,
            });
            let scoped = service.scoped_chunking(&pinned);

            assert_eq!(scoped.chunking().snapshot(), pinned);
            assert_eq!(scoped.library().chunking_runtime().snapshot(), pinned);
            assert_eq!(
                scoped.translation().operation_chunking(),
                Some(TranslationChunkingBudget::from(&pinned)),
                "the scoped task publishes under its own snapshot"
            );
            assert_eq!(
                scoped.library().translation().operation_chunking(),
                Some(TranslationChunkingBudget::from(&pinned)),
                "a reused file converts its translations under the task's snapshot"
            );
            assert_eq!(
                service.library().translation().operation_chunking(),
                Some(TranslationChunkingBudget::from(&installed)),
                "the unscoped library keeps the live budget"
            );
            assert_eq!(
                live.snapshot(),
                installed,
                "the live handle keeps the installed budget"
            );

            let _ = std::fs::remove_dir_all(storage_root);
        }
    }
}
