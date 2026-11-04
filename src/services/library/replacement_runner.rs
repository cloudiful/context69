//! The staged replacement executor (issue #1138 P16).
//!
//! One call takes the freshly parsed sections its future caller prepared,
//! stages their vectors as insert-only points under the durable run's
//! generation, and applies the P15 switch kernel. The old content, source and
//! history stay untouched until the switch commits.
//!
//! The runner is the write path of the ordinary library ingestion: the file
//! upload, URL and text tasks call it through the atomic ingest processors, and
//! it fences the competing metadata and translation writers with the same
//! registry admission and durable run identity. It claims no source freshness
//! itself; the caller owns parsing and staging.
//!
//! Failure shape: a transient embed, upsert or verification failure returns an
//! error and keeps the committed staging admission and the durable per-section
//! document mapping in place, so a retry with the same durable generation
//! reuses the same final document ids, staged ids and admission. Abandoning a
//! run and releasing its source is a later task-lifecycle responsibility and
//! never happens here.

use std::collections::{BTreeSet, HashSet};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, original_chunk, seed};
use crate::chunking::ChunkingConfig;
use crate::db::{Database, IngestRunMode, IngestRunSource, NewIngestRun, RegistryQuotaOverride};
use crate::domain::{ChunkPayload, DocumentChunk, LibraryFileDocumentRecord, LibraryFileRecord};
use crate::embedding::EmbeddingProvider;
use crate::library_store::{LibraryStore, TaskWriteFence, begin_task_write};
use crate::qdrant_index::QdrantIndex;
use crate::qdrant_index::metadata_retirement::OriginalPointExpectation;
use context69_search::SearchRegistryEntry;

use super::ingest_switch::{
    PreparedReplacement, PreparedReplacementSection, ReplacementDocumentBaseline,
    ReplacementFileBaseline, ReplacementSwitchDisposition, switch_replacement,
};
use super::metadata_helpers::{compose_library_metadata, library_system_metadata};
use super::tree::folder_path_for;
use super::{FILE_LIBRARY_SOURCE_KEY, ReplacementFileAttributes, TaskIngestStep, ingest_batches};

/// One freshly parsed section the caller prepared for replacement.
///
/// Only parsed content travels in; the runner composes the final metadata,
/// resolves the authoritative external identity and normalizes the record
/// itself, so no caller-supplied content hash can disagree with what is
/// stored.
#[derive(Debug, Clone)]
pub struct ReplacementSectionInput {
    pub section_key: String,
    pub section_label: String,
    pub sort_order: i32,
    pub section_metadata_json: serde_json::Value,
    pub title: String,
    pub summary: Option<String>,
    pub body_text: String,
    pub source_uri: Option<String>,
    pub external_id: Option<String>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The request of one replacement execution.
#[derive(Debug, Clone)]
pub struct ReplacementRunRequest {
    /// The application task that owns the task item.
    ///
    /// This is the task's own id, never the fence's execution task id. The
    /// database verifies it against the item row inside the fenced claim, so a
    /// wrong or stale task id refuses the run before anything is staged.
    pub task_id: Uuid,
    pub file_id: Uuid,
    pub source: IngestRunSource,
    /// The staged source object attached to the task item, promoted by the
    /// switch. `None` keeps the file's current source.
    pub staged_source_object: Option<Uuid>,
    /// The narrowing of the persisted registry capacity policy this run
    /// admits under.
    ///
    /// The persisted policy is the authority: this value can only lower the
    /// caps the deployment allows, never raise them.
    pub quota: RegistryQuotaOverride,
    /// The projected file attributes of an ordinary update. `None` keeps the
    /// current row attributes, as the fresh-source rerun does.
    pub attributes: Option<ReplacementFileAttributes>,
    pub sections: Vec<ReplacementSectionInput>,
}

/// The external dependencies of one replacement execution.
///
/// Both handles are the caller's own runtime handles, so tests can supply a
/// mock points service and a fake embedder without a [`super::LibraryService`].
pub struct ReplacementVector<'a> {
    pub embedding: &'a dyn EmbeddingProvider,
    pub index: &'a QdrantIndex,
    pub chunking: &'a ChunkingConfig,
    /// The vector identity this run writes its points under.
    ///
    /// The registry admission records it, and the switch compares it with the
    /// persisted configuration. A run without an identity cannot be admitted:
    /// a point written under an unknown vector space must never become
    /// visible.
    pub identity: Option<crate::db::RegistryVectorIdentity>,
}

impl ReplacementVector<'_> {
    /// The identity the registry admission of this run records.
    fn registry_identity(&self) -> Result<&crate::db::RegistryVectorIdentity> {
        self.identity.as_ref().context(
            "the replacement run has no vector identity; the runtime must publish a \
                      configured vector space before it writes points",
        )
    }
}

/// How one replacement execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementRunDisposition {
    /// The replacement was applied by this call.
    Switched,
    /// The durable run was already resolved by an earlier successful switch,
    /// so this call staged nothing and wrote no points.
    AlreadyApplied,
}

/// The observable outcome of one replacement execution.
#[derive(Debug, Clone)]
pub struct ReplacementRunOutcome {
    pub disposition: ReplacementRunDisposition,
    pub run_id: Uuid,
    pub generation: i64,
    /// The final documents of the replacement, in section order. Empty for a
    /// replay, which prepared no plan.
    pub document_ids: Vec<i64>,
    /// The staged point ids that became the live version.
    pub staged_point_ids: Vec<Uuid>,
    /// The point ids of the version that is no longer live. Empty for a replay.
    pub retired_point_ids: Vec<Uuid>,
    /// Retired ids whose physical delete failed or was skipped. The registry
    /// tombstone keeps them excluded; a later pass may delete them.
    pub pending_cleanup_point_ids: Vec<Uuid>,
}

/// Executes one staged library replacement in one unbounded call.
pub async fn run_replacement(
    db: &Database,
    store: &LibraryStore,
    vector: &ReplacementVector<'_>,
    fence: TaskWriteFence,
    request: &ReplacementRunRequest,
) -> Result<ReplacementRunOutcome> {
    match run_atomic(
        db,
        store,
        vector,
        fence,
        request,
        IngestRunMode::Replacement,
        None,
    )
    .await?
    {
        TaskIngestStep::Complete(outcome) => Ok(outcome),
        TaskIngestStep::Yielded => bail!("an unbounded replacement run reached a finite turn"),
    }
}

/// Executes one staged atomic first insert in one unbounded call.
///
/// The runner is the same kernel as the replacement, but the durable run it
/// adopts or claims carries the first-insert mode: the file was created
/// together with that run, so it never carried committed content and no
/// document, point, version or result of it is replaced.
pub async fn run_first_insert(
    db: &Database,
    store: &LibraryStore,
    vector: &ReplacementVector<'_>,
    fence: TaskWriteFence,
    request: &ReplacementRunRequest,
) -> Result<ReplacementRunOutcome> {
    match run_atomic(
        db,
        store,
        vector,
        fence,
        request,
        IngestRunMode::FirstInsert,
        None,
    )
    .await?
    {
        TaskIngestStep::Complete(outcome) => Ok(outcome),
        TaskIngestStep::Yielded => bail!("an unbounded first insert reached a finite turn"),
    }
}

/// One bounded staging turn of a replacement task.
///
/// The same kernel runs, but it commits at most [`STAGING_TURN_BATCHES`]
/// verified batches before returning `Yielded`, so the worker can release its
/// claim and let another scheduled task run.
pub(crate) async fn stage_replacement(
    db: &Database,
    store: &LibraryStore,
    vector: &ReplacementVector<'_>,
    fence: TaskWriteFence,
    request: &ReplacementRunRequest,
    batches_per_segment: usize,
) -> Result<TaskIngestStep<ReplacementRunOutcome>> {
    run_atomic(
        db,
        store,
        vector,
        fence,
        request,
        IngestRunMode::Replacement,
        Some(batches_per_segment),
    )
    .await
}

/// One bounded staging turn of an atomic first insert.
pub(crate) async fn stage_first_insert(
    db: &Database,
    store: &LibraryStore,
    vector: &ReplacementVector<'_>,
    fence: TaskWriteFence,
    request: &ReplacementRunRequest,
    batches_per_segment: usize,
) -> Result<TaskIngestStep<ReplacementRunOutcome>> {
    run_atomic(
        db,
        store,
        vector,
        fence,
        request,
        IngestRunMode::FirstInsert,
        Some(batches_per_segment),
    )
    .await
}

/// The shared execution of one atomic library ingest.
///
/// The durable run is the authority of the mode: an existing run is adopted
/// with the mode it was claimed with, and a missing run is claimed with the
/// mode of the caller. The switch refuses a run whose stored mode differs, so
/// a first insert is never applied as a replacement.
///
/// `batch_limit` is the finite turn quantum of a scheduled task segment.
/// `None` is the unbounded mode of the direct complete wrappers.
async fn run_atomic(
    db: &Database,
    store: &LibraryStore,
    vector: &ReplacementVector<'_>,
    fence: TaskWriteFence,
    request: &ReplacementRunRequest,
    mode: IngestRunMode,
    batch_limit: Option<usize>,
) -> Result<TaskIngestStep<ReplacementRunOutcome>> {
    validate_request(request)?;
    if let Some(limit) = batch_limit {
        super::stage_quantum::validate_finite_batches_per_segment(limit)?;
    }
    if mode == IngestRunMode::FirstInsert {
        ensure!(
            request.attributes.is_none(),
            "a first insert publishes the attributes of the created file"
        );
        ensure!(
            request.staged_source_object.is_none(),
            "a first insert reads the source of the created file and promotes nothing"
        );
    }
    let file = store.get_file(request.file_id).await?.ok_or_else(|| {
        anyhow::anyhow!("the replacement file {} no longer exists", request.file_id)
    })?;

    // The durable run is claimed or adopted on the item's fenced connection;
    // the database verifies the application task id, the file and the grant.
    let run = {
        let mut write = begin_task_write(db, fence).await?;
        let existing = db.read_library_ingest_run_in(&mut write).await?;
        let requested = match existing {
            Some(existing) => {
                if existing.mode != mode {
                    return Err(staged_plan_refusal(format!(
                        "the durable run {} was claimed in another mode",
                        existing.batch_id
                    )));
                }
                None
            }
            None => Some(NewIngestRun {
                mode,
                source: request.source,
            }),
        };
        let run = db
            .claim_or_create_library_ingest_run_in(
                &mut write,
                request.task_id,
                request.file_id,
                requested,
            )
            .await?;
        let committed = write.commit().await?;
        ensure!(
            committed,
            "the grant no longer authorizes the replacement claim"
        );
        run
    };
    if run.resolved_at.is_some() {
        // A replay of a successfully switched run never stages and never
        // writes points; the file keeps the state the first switch applied.
        return Ok(TaskIngestStep::Complete(ReplacementRunOutcome {
            disposition: ReplacementRunDisposition::AlreadyApplied,
            run_id: run.batch_id,
            generation: run.generation,
            document_ids: Vec::new(),
            staged_point_ids: Vec::new(),
            retired_point_ids: Vec::new(),
            pending_cleanup_point_ids: Vec::new(),
        }));
    }

    let plan = prepare_plan(db, store, fence, &file, &run, request, vector).await?;

    // The deterministic plan identity and cursor are validated before any
    // admission mutation, completed-prefix skip or vector write. A changed
    // plan, a foreign run or a malformed cursor is a deterministic refusal:
    // retrying against the same immutable staged set can never succeed.
    let batches = staged_batches(&plan);
    let total_batches =
        i64::try_from(batches.len()).context("the staging batch count overflows")?;
    let fingerprint = plan_fingerprint(&plan)?;
    let progress = db
        .read_library_ingest_run_stage_progress(run.item_id)
        .await?;
    let view = progress.as_ref().map(|progress| StageProgressView {
        batch_id: progress.batch_id,
        generation: progress.generation,
        staged_batches_written: progress.staged_batches_written,
        staged_plan_fingerprint: progress.staged_plan_fingerprint.as_deref(),
    });
    let already_written = validate_stage_progress(view, &run, total_batches, &fingerprint)?;

    ensure_admission(
        db,
        fence,
        &run,
        &plan,
        request.quota,
        vector.registry_identity()?,
    )
    .await?;

    let mut committed_this_segment = 0usize;
    for (index, batch) in batches.iter().enumerate() {
        if index < already_written {
            continue;
        }
        write_staged_batch(vector, &plan, batch).await?;
        verify_staged_batch(vector, &plan, batch).await?;
        let expected_previous =
            i64::try_from(index).context("the staging batch index overflows")?;
        ensure!(
            db.advance_library_ingest_run_stage_progress(
                fence,
                &run,
                expected_previous,
                &fingerprint,
            )
            .await?,
            "the staging progress advance was refused; no switch is applied"
        );
        committed_this_segment += 1;
        if batch_limit.is_some_and(|limit| {
            super::stage_quantum::finite_turn_should_yield(
                committed_this_segment,
                limit,
                index + 1,
                batches.len(),
            )
        }) {
            return Ok(TaskIngestStep::Yielded);
        }
    }

    // The completed prefix was verified batch by batch; the full staged set is
    // verified once more before the single atomic switch publishes it.
    verify_staged_points(vector, &plan).await?;

    let outcome = switch_replacement(db, store, fence, &plan.replacement).await?;
    let retired_point_ids = outcome.retired_point_ids.clone();
    let pending =
        cleanup_retired_points(db, vector.index, &retired_point_ids, &plan.staged_point_ids).await;

    Ok(TaskIngestStep::Complete(ReplacementRunOutcome {
        disposition: match outcome.disposition {
            ReplacementSwitchDisposition::Switched => ReplacementRunDisposition::Switched,
            ReplacementSwitchDisposition::AlreadyApplied => {
                ReplacementRunDisposition::AlreadyApplied
            }
        },
        run_id: plan.replacement.run_id,
        generation: run.generation,
        document_ids: outcome.document_ids,
        staged_point_ids: plan.staged_point_ids,
        retired_point_ids,
        pending_cleanup_point_ids: pending,
    }))
}

/// A deterministic staged-plan refusal.
///
/// These failures describe an immutable staged set or run identity that a
/// retry cannot change, so they are classified through the existing typed
/// deterministic taxonomy instead of a retryable generic error.
fn staged_plan_refusal(message: impl Into<String>) -> anyhow::Error {
    crate::domain_errors::DomainError::invalid_argument(message).into()
}

/// The persisted staging fields the kernel validates, without the private
/// database row type.
struct StageProgressView<'a> {
    batch_id: Uuid,
    generation: i64,
    staged_batches_written: i64,
    staged_plan_fingerprint: Option<&'a str>,
}

/// Validates the persisted staging cursor against the current plan identity.
///
/// The check runs before admission or any staged write so a changed plan can
/// never reuse an immutable admission and retry forever. It returns the number
/// of already committed batches only when the cursor and fingerprint agree.
fn validate_stage_progress(
    progress: Option<StageProgressView<'_>>,
    run: &crate::db::LibraryIngestRun,
    total_batches: i64,
    fingerprint: &str,
) -> Result<usize> {
    let progress = progress.ok_or_else(|| {
        staged_plan_refusal("the current durable ingest run has no staging progress")
    })?;
    if progress.batch_id != run.batch_id || progress.generation != run.generation {
        return Err(staged_plan_refusal(
            "the staging progress belongs to another durable run",
        ));
    }
    if progress.staged_batches_written < 0 || progress.staged_batches_written > total_batches {
        return Err(staged_plan_refusal(format!(
            "the staging progress {} is outside the plan length {total_batches}",
            progress.staged_batches_written
        )));
    }
    match progress.staged_plan_fingerprint.as_deref() {
        Some(stored) if stored == fingerprint => {}
        Some(_) => {
            return Err(staged_plan_refusal(
                "the staged plan changed since the last checkpoint; refusing to skip completed work",
            ));
        }
        None if progress.staged_batches_written == 0 => {}
        None => {
            return Err(staged_plan_refusal(
                "the staging progress has no plan fingerprint",
            ));
        }
    }
    usize::try_from(progress.staged_batches_written)
        .map_err(|_| staged_plan_refusal("the staging progress does not fit in memory"))
}

/// The internal consistency of one request, before any database access.
fn validate_request(request: &ReplacementRunRequest) -> Result<()> {
    if request.sections.is_empty() {
        bail!("a replacement requires at least one section");
    }
    let mut keys = BTreeSet::new();
    for section in &request.sections {
        ensure!(
            !section.section_key.trim().is_empty(),
            "a replacement section key must not be empty"
        );
        ensure!(
            keys.insert(section.section_key.as_str()),
            "the replacement section key {} appears twice",
            section.section_key
        );
    }
    Ok(())
}

/// The prepared plan of one replacement: the staged points, the switch plan
/// and the registry entry that admits the staged set.
struct ReplacementPlan {
    replacement: PreparedReplacement,
    registry_entry: SearchRegistryEntry,
    staged_point_ids: Vec<Uuid>,
    anticipated_retired_count: i64,
}

/// Prepares the stable plan of a replacement: document identities, baselines,
/// generation-scoped chunks, payloads and links.
///
/// The identity and baseline reads run on short-lived pooled connections; the
/// durable section reservations and the final admission hold the captured task
/// grant, so a terminal retirement either commits before them and refuses them
/// or waits for their transaction. The plan is re-validated by the switch
/// against the rows it locks.
async fn prepare_plan(
    db: &Database,
    store: &LibraryStore,
    fence: TaskWriteFence,
    file: &LibraryFileRecord,
    run: &crate::db::LibraryIngestRun,
    request: &ReplacementRunRequest,
    vector: &ReplacementVector<'_>,
) -> Result<ReplacementPlan> {
    let projected_file = request
        .attributes
        .as_ref()
        .map(|attributes| attributes.project(file))
        .unwrap_or_else(|| file.clone());
    let folder_path = folder_path_for(store, projected_file.folder_id).await?;
    let retained = store.list_file_documents(request.file_id).await?;

    let mut sections: Vec<PreparedReplacementSection> = Vec::new();
    let mut baselines: Vec<ReplacementDocumentBaseline> = Vec::new();
    let mut document_ids: Vec<i64> = Vec::new();

    for (index, input) in request.sections.iter().enumerate() {
        // The metadata composition mirrors the ordinary ingest: the section
        // metadata, then the file metadata, then the system keys that cannot
        // be forged.
        let metadata_json = compose_library_metadata(
            &input.section_metadata_json,
            &projected_file.metadata_json,
            library_system_metadata(
                &projected_file,
                &folder_path,
                &input.section_key,
                &input.section_label,
            ),
        )
        .context("compose the replacement section metadata")?;

        // A section the file already links is matched by its retained
        // section mapping first; its document id and external identity are
        // authoritative and never remapped when the section order changes.
        let retained_link = retained
            .iter()
            .find(|row| row.section_key == input.section_key);
        let derived = external_id_for(&projected_file, &request.sections, index);
        let document_id = match retained_link {
            Some(link) => link.document_id,
            None => match db
                .find_document_id_by_key(file.group_id, FILE_LIBRARY_SOURCE_KEY, &derived)
                .await?
            {
                Some(id) => id,
                // No committed document carries the section identity yet, so
                // the run reserves its final id and records that decision
                // durably before the first external point write. A retry of
                // the same run therefore reuses the exact id instead of
                // reserving a new one.
                None => {
                    db.reserve_library_ingest_run_section(fence, run, &input.section_key, &derived)
                        .await?
                }
            },
        };
        ensure!(
            !document_ids.contains(&document_id),
            "the replacement sections resolve to the document {} twice",
            document_id
        );
        document_ids.push(document_id);

        let mut connection = db.pool().acquire().await?;
        let mut locked = db
            .lock_replacement_documents_on(&mut connection, &[document_id])
            .await?;
        drop(connection);
        let snapshot = locked
            .iter_mut()
            .find(|document| document.id == document_id);
        // A retained section keeps the stored `(source_key, external_id)`
        // identity of its document, so the switch updates the same row and the
        // generation-scoped chunk ids stay stable across retries. A new
        // section is created under the ordinary library source key.
        let (source_key, external_id, baseline_hash) = match (&retained_link, snapshot) {
            (Some(_), Some(document)) => (
                document.source_key.clone(),
                document.external_id.clone(),
                Some(document.record_hash.clone()),
            ),
            (Some(_), None) => bail!(
                "the retained section {} links the document {} which no longer exists",
                input.section_key,
                document_id
            ),
            (None, Some(document)) => (
                document.source_key.clone(),
                document.external_id.clone(),
                Some(document.record_hash.clone()),
            ),
            (None, None) => (FILE_LIBRARY_SOURCE_KEY.to_string(), derived, None),
        };
        if let Some(hash) = &baseline_hash {
            baselines.push(ReplacementDocumentBaseline {
                document_id,
                record_hash: hash.clone(),
            });
        }

        // The final stored content is normalized and hashed together: the
        // composed metadata and the authoritative external identity are part
        // of the hashed record, so a stale caller hash can never disagree with
        // what the document row and the point payloads carry.
        let normalized = crate::normalize::normalize_record(crate::domain::SourceRecord {
            external_id,
            title: input.title.clone(),
            body_text: input.body_text.clone(),
            source_uri: projected_file
                .source_uri
                .clone()
                .or_else(|| input.source_uri.clone())
                .unwrap_or_else(|| format!("context69://library/files/{}", file.id)),
            summary: input.summary.clone(),
            published_at: projected_file.published_at.or(input.published_at),
            // The source update timestamp is part of the staged payload and
            // therefore of the plan fingerprint. The file row's update time is
            // stable across the claims of one run; a changing row is a real
            // payload change and refuses the prefix instead of silently
            // rewriting it.
            updated_at: projected_file.updated_at,
            metadata_json,
        });

        let chunks: Vec<DocumentChunk> = crate::chunking::chunk_document_iter(
            document_id,
            &source_key,
            &normalized,
            vector.chunking,
            Some(run.generation),
        )
        .collect();
        let payload = seed(
            &PayloadGroup {
                group_id: file.group_id,
                group_key: &file.group_key,
                group_path: &file.group_path,
                visibility: file.visibility,
            },
            &PayloadDocument {
                source_key: source_key.as_str(),
                external_id: &normalized.external_id,
                title: &normalized.title,
                summary: normalized.summary.as_deref(),
                source_uri: &normalized.source_uri,
                published_at: normalized.published_at,
                updated_at_source: normalized.updated_at,
                record_hash: &normalized.record_hash,
                metadata_json: &normalized.metadata_json,
            },
            &normalized.body_text,
        );
        let link = LibraryFileDocumentRecord {
            file_id: request.file_id,
            document_id,
            group_id: file.group_id,
            visibility: file.visibility,
            section_key: input.section_key.clone(),
            section_label: input.section_label.clone(),
            // The section identity metadata of a retained section is
            // preserved; only the freshly parsed content moves.
            section_external_id: retained_link.and_then(|row| row.section_external_id.clone()),
            section_source_uri: retained_link.and_then(|row| row.section_source_uri.clone()),
            section_published_at: retained_link.and_then(|row| row.section_published_at),
            section_metadata_json: input.section_metadata_json.clone(),
            sort_order: input.sort_order,
        };
        sections.push(PreparedReplacementSection {
            document_id,
            payload,
            chunks,
            link,
        });
    }

    let baseline = file_baseline(db, store, request.file_id).await?;

    let mut staged_ids_seen: HashSet<Uuid> = HashSet::new();
    let mut staged_point_ids: Vec<Uuid> = Vec::new();
    for section in &sections {
        for chunk in &section.chunks {
            ensure!(
                staged_ids_seen.insert(chunk.id),
                "the staged point {} appears twice",
                chunk.id
            );
            staged_point_ids.push(chunk.id);
            ensure!(
                chunk.text.chars().count() <= ingest_batches::MAX_BATCH_CHARS,
                "the staged chunk {} exceeds the embedding batch budget",
                chunk.id
            );
        }
    }
    ensure!(
        !staged_point_ids.is_empty(),
        "the replacement staged set is empty"
    );

    // The anticipated retired set uses the same definition as the switch: the
    // document's own chunks plus every stored translation whose version does
    // not match the source hash the replacement publishes.
    let mut anticipated_retired: Vec<Uuid> = Vec::new();
    for baseline_document in &baselines {
        let new_hash = sections
            .iter()
            .find(|section| section.document_id == baseline_document.document_id)
            .map(|section| section.payload.record_hash.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "the affected document {} has no prepared section",
                    baseline_document.document_id
                )
            })?;
        let mut connection = db.pool().acquire().await?;
        let ids = db
            .replacement_point_ids_on(
                &mut connection,
                baseline_document.document_id,
                new_hash,
                true,
            )
            .await?;
        drop(connection);
        anticipated_retired.extend(ids);
    }
    let mut seen = HashSet::new();
    anticipated_retired.retain(|id| seen.insert(*id));
    let anticipated_retired_count = i64::try_from(anticipated_retired.len())
        .context("the anticipated retired count overflows")?;

    let registry_entry = SearchRegistryEntry {
        run_id: run.batch_id,
        file_id: Some(request.file_id),
        document_ids: document_ids.clone(),
        staged_ids: staged_point_ids.clone(),
        retired_ids: Vec::new(),
        baseline_record_hash: None,
        staged_record_hash: None,
        generation: run.generation,
        phase: "staging".into(),
    };

    let replacement = PreparedReplacement {
        run_id: run.batch_id,
        file_id: request.file_id,
        mode: run.mode,
        source: request.source,
        baseline,
        baseline_documents: baselines,
        sections,
        attributes: request.attributes.clone(),
        staged_source_object: request.staged_source_object,
    };

    Ok(ReplacementPlan {
        replacement,
        registry_entry,
        staged_point_ids,
        anticipated_retired_count,
    })
}

/// The external id the ordinary ingest derives for one section position.
///
/// The derivation mirrors `prepare_sections`: the file's own external id keys
/// the first section, the following sections carry the file base with their
/// section key, and an unsuffixed file falls back to the normalized external
/// id, then to the file id plus the section key.
fn external_id_for(
    file: &LibraryFileRecord,
    sections: &[ReplacementSectionInput],
    index: usize,
) -> String {
    let section = &sections[index];
    file.external_id
        .as_ref()
        .map(|base| {
            if index == 0 {
                base.clone()
            } else {
                format!("{base}:{}", section.section_key)
            }
        })
        .or_else(|| section.external_id.clone())
        .unwrap_or_else(|| format!("{}:{}", file.id, section.section_key))
}

/// The locked file baseline the replacement was prepared against.
async fn file_baseline(
    db: &Database,
    store: &LibraryStore,
    file_id: Uuid,
) -> Result<ReplacementFileBaseline> {
    let mut connection = db.pool().acquire().await?;
    let snapshot = store
        .lock_replacement_file_on(&mut connection, file_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the library file {file_id} no longer exists"))?;
    drop(connection);
    Ok(ReplacementFileBaseline {
        group_id: snapshot.group_id,
        folder_id: snapshot.folder_id,
        external_id: snapshot.external_id,
        source_uri: snapshot.source_uri,
        published_at: snapshot.published_at,
        metadata_json: snapshot.metadata_json,
        filename: snapshot.filename,
        media_type: snapshot.media_type,
        size_bytes: snapshot.size_bytes,
        sha256: snapshot.sha256,
        storage_rel_path: snapshot.storage_rel_path,
        storage_object_id: snapshot.storage_object_id,
        source_released_at: snapshot.source_released_at,
        updated_at: snapshot.updated_at,
    })
}

/// Commits the bounded registry admission of the complete staged set, or
/// reuses an identical staging admission from a failed earlier attempt.
///
/// The admission runs inside the captured task grant and holds the item lock,
/// so a terminal retirement either committed before this call and refuses the
/// admission, or waits for this transaction; a retired run can therefore never
/// acquire a fresh staging reservation. The admission still commits before the
/// first point write, and a refusal leaves the database and the collection
/// untouched.
async fn ensure_admission(
    db: &Database,
    fence: TaskWriteFence,
    run: &crate::db::LibraryIngestRun,
    plan: &ReplacementPlan,
    quota: RegistryQuotaOverride,
    identity: &crate::db::RegistryVectorIdentity,
) -> Result<()> {
    let entry = &plan.registry_entry;
    let staged: BTreeSet<Uuid> = entry.staged_ids.iter().copied().collect();
    let documents: BTreeSet<i64> = entry.document_ids.iter().copied().collect();
    let mut write = begin_task_write(db, fence).await?;
    // The held item lock serializes this check with the terminal retirement
    // trigger, so the run this plan was prepared against is still current.
    db.verify_current_library_ingest_run_in(&mut write, run)
        .await?;
    if let Some(existing) = db
        .read_search_registry_entry_in(write.connection(), entry.run_id)
        .await?
    {
        let reused = (|| {
            if existing.phase != "staging" {
                return Err(staged_plan_refusal(format!(
                    "the replacement run {} is already {} and cannot be staged again",
                    entry.run_id, existing.phase
                )));
            }
            if existing.generation != entry.generation {
                return Err(staged_plan_refusal(
                    "the stored staging admission has another generation",
                ));
            }
            if existing.file_id != entry.file_id {
                return Err(staged_plan_refusal(
                    "the stored staging admission belongs to another file",
                ));
            }
            if existing.staged_ids.iter().copied().collect::<BTreeSet<_>>() != staged {
                return Err(staged_plan_refusal(
                    "the stored staging admission holds another staged set",
                ));
            }
            if existing
                .document_ids
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                != documents
            {
                return Err(staged_plan_refusal(
                    "the stored staging admission holds another document set",
                ));
            }
            Ok(())
        })();
        if let Err(error) = reused {
            write.rollback().await;
            return Err(error);
        }
        ensure!(
            write.commit().await?,
            "the grant no longer authorizes the staging admission"
        );
        return Ok(());
    }
    // The captured identity must still describe the persisted configuration.
    // The persisted rows are the authority: a node whose pair predates a
    // settings change on another node must not stage points under the older
    // identity.
    if let Some(persisted) = db
        .read_persisted_vector_identity_in(write.connection())
        .await?
        && persisted != *identity
    {
        write.rollback().await;
        return Err(crate::domain_errors::DomainError::conflict(
            "the configured vector identity changed since the replacement captured it; retry it \
             under the current configuration",
        )
        .into());
    }
    let admitted = db
        .admit_search_registry_entry_checked_in(
            entry,
            Some(identity),
            plan.anticipated_retired_count,
            quota,
            write.connection(),
        )
        .await?;
    match admitted {
        crate::db::AdmissionOutcome::Admitted(_) => {}
        crate::db::AdmissionOutcome::CapsRefused => {
            write.rollback().await;
            // The registry capacity is a policy of the deployment, not a
            // temporary condition: the refusal is deterministic, so the run
            // stops instead of retrying against the same policy. The retained
            // entries are cumulative capacity, so reclaiming means deleting
            // the entities whose points are no longer wanted, or accepting the
            // cost of a larger configured capacity.
            return Err(crate::domain_errors::DomainError::invalid_argument(
                "the registry capacity policy does not allow this replacement; increase the \
                 configured capacity only after evaluating the cost of the retained points, or \
                 delete the entities whose points should no longer be retained",
            )
            .into());
        }
        crate::db::AdmissionOutcome::RebuildBusy => {
            write.rollback().await;
            return Err(crate::domain_errors::DomainError::conflict(
                "a vector index rebuild is in progress; retry the replacement after it completes",
            )
            .into());
        }
    }
    ensure!(
        write.commit().await?,
        "the grant no longer authorizes the registry admission"
    );
    Ok(())
}

/// The exact upsert payloads of one staged batch.
///
/// The section payload mirrors the document row fields; the point payload is
/// built from the same fields per chunk, so the verification compares the
/// identity the document row will carry after the switch.
fn staged_batch_payloads(
    plan: &ReplacementPlan,
    batch: &[&DocumentChunk],
) -> Result<Vec<ChunkPayload>> {
    batch
        .iter()
        .map(|chunk| {
            let section = plan
                .replacement
                .sections
                .iter()
                .find(|section| section.document_id == chunk.document_id)
                .with_context(|| {
                    format!(
                        "the staged chunk {} belongs to no planned section",
                        chunk.id
                    )
                })?;
            Ok(original_chunk(
                &PayloadGroup {
                    group_id: section.payload.group_id,
                    group_key: &section.payload.group_key,
                    group_path: &section.payload.group_path,
                    visibility: section.payload.visibility,
                },
                &PayloadDocument {
                    source_key: &section.payload.source_key,
                    external_id: &section.payload.external_id,
                    title: &section.payload.title,
                    summary: section.payload.summary.as_deref(),
                    source_uri: &section.payload.source_uri,
                    published_at: section.payload.published_at,
                    updated_at_source: section.payload.updated_at_source,
                    record_hash: &section.payload.record_hash,
                    metadata_json: &section.payload.metadata_json,
                },
                ChunkRef {
                    chunk_id: chunk.id,
                    document_id: chunk.document_id,
                    chunk_index: chunk.chunk_index,
                    chunk_text: &chunk.text,
                },
            ))
        })
        .collect()
}

/// Embeds and writes one staged batch in insert-only mode.
///
/// No connection is held across the embedding or the Qdrant call. A failure
/// leaves the committed staging admission and the previous progress cursor in
/// place, so a retry repeats only the uncommitted batch.
async fn write_staged_batch(
    vector: &ReplacementVector<'_>,
    plan: &ReplacementPlan,
    batch: &[&DocumentChunk],
) -> Result<()> {
    let payloads = staged_batch_payloads(plan, batch)?;
    let texts: Vec<String> = batch.iter().map(|chunk| chunk.text.clone()).collect();
    let embeddings = vector.embedding.embed_texts(&texts).await?;
    ensure!(
        embeddings.len() == payloads.len(),
        "the embedding provider returned the wrong vector count"
    );
    vector
        .index
        .upsert_document_chunks_insert_only(&payloads, &embeddings)
        .await
}

/// Verifies one just-written batch by its exact document and record identity.
async fn verify_staged_batch(
    vector: &ReplacementVector<'_>,
    plan: &ReplacementPlan,
    batch: &[&DocumentChunk],
) -> Result<()> {
    let expectations = batch
        .iter()
        .map(|chunk| {
            let section = plan
                .replacement
                .sections
                .iter()
                .find(|section| section.document_id == chunk.document_id)
                .with_context(|| {
                    format!(
                        "the staged chunk {} belongs to no planned section",
                        chunk.id
                    )
                })?;
            Ok(OriginalPointExpectation {
                point_id: chunk.id,
                document_id: section.document_id,
                record_hash: section.payload.record_hash.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    vector.index.verify_original_points(&expectations).await
}

/// The deterministic identity of one whole prepared staging plan.
///
/// It binds the ordered batches, the length-prefixed embedding text, the exact
/// upsert payload and the point id, so a changed source, chunking or metadata
/// cannot silently skip a completed prefix. No raw document or credential is
/// persisted: the digest is one SHA-256 string.
fn plan_fingerprint(plan: &ReplacementPlan) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"context69.library_ingest_stage_plan.v1\0");
    for (batch_index, batch) in staged_batches(plan).iter().enumerate() {
        hasher.update(b"batch\0");
        hasher.update((batch_index as u64).to_be_bytes());
        let payloads = staged_batch_payloads(plan, batch)?;
        for (chunk, payload) in batch.iter().zip(payloads.iter()) {
            hasher.update(b"point\0");
            hasher.update(chunk.id.as_bytes());
            hasher.update(b"text\0");
            let text = chunk.text.as_bytes();
            hasher.update((text.len() as u64).to_be_bytes());
            hasher.update(text);
            hasher.update(b"payload\0");
            let encoded = serde_json::to_vec(&fingerprint_payload(payload))
                .context("serialize the staging fingerprint payload")?;
            hasher.update((encoded.len() as u64).to_be_bytes());
            hasher.update(&encoded);
        }
        hasher.update(b"end-batch\0");
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The stable serialization of the payload fields bound by the fingerprint.
fn fingerprint_payload(payload: &ChunkPayload) -> serde_json::Value {
    serde_json::json!({
        "chunk_id": payload.chunk_id.to_string(),
        "document_id": payload.document_id,
        "group_id": payload.group_id,
        "group_key": &payload.group_key,
        "group_path": &payload.group_path,
        "visibility": payload.visibility.as_str(),
        "source_key": &payload.source_key,
        "external_id": &payload.external_id,
        "title": &payload.title,
        "summary": payload.summary.as_deref(),
        "source_uri": &payload.source_uri,
        "published_at": payload.published_at.map(|value| value.to_rfc3339()),
        "updated_at_source": payload.updated_at_source.to_rfc3339(),
        "record_hash": &payload.record_hash,
        "chunk_index": payload.chunk_index,
        "chunk_text": &payload.chunk_text,
        "metadata_json": &payload.metadata_json,
        "content_locale": &payload.content_locale,
        "source_locale": payload.source_locale.as_deref(),
        "translation_provider": payload.translation_provider.as_deref(),
    })
}

/// The embedding batches of one plan: one batch per section while the section
/// fits the existing budget, chunk-sized batches for an oversized section.
fn staged_batches(plan: &ReplacementPlan) -> Vec<Vec<&DocumentChunk>> {
    let mut batches = Vec::new();
    for section in &plan.replacement.sections {
        let section_chars: usize = section
            .chunks
            .iter()
            .map(|chunk| chunk.text.chars().count())
            .sum();
        if section_chars <= ingest_batches::MAX_BATCH_CHARS {
            if !section.chunks.is_empty() {
                batches.push(section.chunks.iter().collect());
            }
        } else {
            for chunk in &section.chunks {
                batches.push(vec![chunk]);
            }
        }
    }
    batches
}

/// Verifies every staged point by its exact document and record identity.
///
/// A successful get RPC alone is not enough: the stored payload must name the
/// planned document and record hash, so a foreign point under the same id
/// refuses the switch.
async fn verify_staged_points(
    vector: &ReplacementVector<'_>,
    plan: &ReplacementPlan,
) -> Result<()> {
    let expectations = plan
        .replacement
        .sections
        .iter()
        .flat_map(|section| {
            section.chunks.iter().map(|chunk| OriginalPointExpectation {
                point_id: chunk.id,
                document_id: section.document_id,
                record_hash: section.payload.record_hash.clone(),
            })
        })
        .collect::<Vec<_>>();
    vector.index.verify_original_points(&expectations).await
}

/// Deletes the physically retired points best effort and reports what is left.
///
/// A failure never fails the switch and never touches the registry tombstone.
/// A staged id of any in-flight run is never deleted, so the new live version
/// can never lose a point to this cleanup.
async fn cleanup_retired_points(
    db: &Database,
    index: &QdrantIndex,
    retired_point_ids: &[Uuid],
    just_staged_point_ids: &[Uuid],
) -> Vec<Uuid> {
    let mut protected: HashSet<Uuid> = just_staged_point_ids.iter().copied().collect();
    if let Ok(state) = db.read_search_registry_state().await {
        for entry in state
            .entries
            .iter()
            .filter(|entry| entry.phase == "staging")
        {
            protected.extend(entry.staged_ids.iter().copied());
        }
    }
    let deletable: Vec<Uuid> = retired_point_ids
        .iter()
        .copied()
        .filter(|id| !protected.contains(id))
        .collect();
    if deletable.is_empty() {
        return retired_point_ids.to_vec();
    }
    match index.delete_points(&deletable).await {
        Ok(()) => retired_point_ids
            .iter()
            .copied()
            .filter(|id| protected.contains(id))
            .collect(),
        Err(_) => deletable,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ReplacementRunRequest, ReplacementSectionInput, fingerprint_payload, validate_request,
        validate_stage_progress,
    };
    use crate::db::{IngestRunMode, IngestRunSource, LibraryIngestRun, RegistryQuotaOverride};
    use crate::domain::ChunkPayload;
    use crate::retry_policy::{FailureClass, classify};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn section(key: &str) -> ReplacementSectionInput {
        ReplacementSectionInput {
            section_key: key.into(),
            section_label: key.into(),
            sort_order: 0,
            section_metadata_json: serde_json::json!({}),
            title: "title".into(),
            summary: None,
            body_text: "body".into(),
            source_uri: Some("context69://test".into()),
            external_id: Some("external".into()),
            published_at: None,
        }
    }

    fn request(sections: Vec<ReplacementSectionInput>) -> ReplacementRunRequest {
        ReplacementRunRequest {
            task_id: Uuid::new_v4(),
            file_id: Uuid::new_v4(),
            source: crate::db::IngestRunSource::Stored,
            staged_source_object: None,
            quota: RegistryQuotaOverride::exact([1000, 1000, 1000]),
            attributes: None,
            sections,
        }
    }

    fn fingerprint_payload_sample() -> ChunkPayload {
        ChunkPayload {
            chunk_id: Uuid::nil(),
            document_id: 7,
            group_id: 3,
            group_key: "group".into(),
            group_path: "test/group".into(),
            visibility: context69_contracts::Visibility::Private,
            source_key: "library".into(),
            external_id: "external".into(),
            title: "title".into(),
            summary: None,
            source_uri: "context69://test".into(),
            published_at: None,
            updated_at_source: Utc::now(),
            record_hash: "a".repeat(64),
            chunk_index: 0,
            chunk_text: "body".into(),
            metadata_json: json!({"k": 1}),
            content_locale: "original".into(),
            source_locale: None,
            translation_provider: None,
        }
    }

    #[test]
    fn a_finite_turn_yields_only_after_its_configured_quantum() {
        use super::super::stage_quantum::{
            finite_turn_should_yield, validate_finite_batches_per_segment,
        };

        for limit in [1usize, 2, 64] {
            assert_eq!(
                validate_finite_batches_per_segment(limit).expect("the quantum is valid"),
                limit
            );
            assert!(
                finite_turn_should_yield(limit, limit, limit, limit + 1),
                "quantum {limit} yields once it committed its batches while work remains"
            );
            assert!(
                !finite_turn_should_yield(limit, limit, limit + 1, limit + 1),
                "quantum {limit} never spends an empty trailing turn"
            );
            assert!(!finite_turn_should_yield(
                limit - 1,
                limit,
                limit,
                limit + 1
            ));
        }
    }

    #[test]
    fn an_invalid_finite_quantum_is_a_deterministic_refusal() {
        use super::super::stage_quantum::validate_finite_batches_per_segment;
        use crate::retry_policy::{FailureClass, classify};

        for limit in [0usize, 65, usize::MAX] {
            let error = validate_finite_batches_per_segment(limit)
                .expect_err("an invalid finite quantum must be refused");
            assert_eq!(classify(&error), FailureClass::Deterministic);
        }
    }

    fn stage_run(batch_id: Uuid, generation: i64) -> LibraryIngestRun {
        LibraryIngestRun {
            item_id: Uuid::nil(),
            task_id: Uuid::nil(),
            file_id: Uuid::nil(),
            batch_id,
            generation,
            mode: IngestRunMode::Replacement,
            source: IngestRunSource::Stored,
            lease_token: Uuid::nil(),
            execution_task_id: Uuid::nil(),
            execution_generation: generation,
            resolved_at: None,
            retired_at: None,
        }
    }

    fn stage_progress<'a>(
        batch_id: Uuid,
        generation: i64,
        written: i64,
        fingerprint: Option<&'a str>,
    ) -> super::StageProgressView<'a> {
        super::StageProgressView {
            batch_id,
            generation,
            staged_batches_written: written,
            staged_plan_fingerprint: fingerprint,
        }
    }

    #[test]
    fn a_changed_staged_plan_is_a_deterministic_refusal() {
        let batch_id = Uuid::new_v4();
        let run = stage_run(batch_id, 1);
        let progress = stage_progress(batch_id, 1, 1, Some("b"));
        let error = validate_stage_progress(Some(progress), &run, 3, &"a".repeat(64))
            .expect_err("a changed plan must be refused");
        assert!(error.to_string().contains("staged plan changed"));
        assert_eq!(classify(&error), FailureClass::Deterministic);
    }

    #[test]
    fn malformed_stage_progress_is_a_deterministic_refusal() {
        let batch_id = Uuid::new_v4();
        let run = stage_run(batch_id, 1);
        let owned_fingerprint = "a".repeat(64);
        let fingerprint = owned_fingerprint.as_str();
        let cases = [
            None,
            Some(stage_progress(Uuid::new_v4(), 1, 0, None)),
            Some(stage_progress(batch_id, 2, 0, None)),
            Some(stage_progress(batch_id, 1, 4, Some(fingerprint))),
            Some(stage_progress(batch_id, 1, 1, None)),
        ];
        for progress in cases {
            let error = validate_stage_progress(progress, &run, 3, fingerprint)
                .expect_err("malformed progress must be refused");
            assert_eq!(
                classify(&error),
                FailureClass::Deterministic,
                "a malformed cursor must not retry against an immutable plan: {error}"
            );
        }
    }

    #[test]
    fn matching_stage_progress_returns_the_committed_prefix() {
        let batch_id = Uuid::new_v4();
        let run = stage_run(batch_id, 1);
        let owned_fingerprint = "a".repeat(64);
        let fingerprint = owned_fingerprint.as_str();
        let zero = stage_progress(batch_id, 1, 0, None);
        assert_eq!(
            validate_stage_progress(Some(zero), &run, 3, fingerprint).expect("zero is valid"),
            0
        );
        let resumed = stage_progress(batch_id, 1, 2, Some(fingerprint));
        assert_eq!(
            validate_stage_progress(Some(resumed), &run, 3, fingerprint).expect("prefix is valid"),
            2
        );
    }

    #[test]
    fn the_fingerprint_payload_is_deterministic_and_binds_content_and_metadata() {
        let base = fingerprint_payload_sample();
        let first = serde_json::to_vec(&fingerprint_payload(&base)).expect("serialize");
        let second = serde_json::to_vec(&fingerprint_payload(&base)).expect("serialize");
        assert_eq!(first, second, "the same payload serializes identically");

        let mut changed_text = base.clone();
        changed_text.chunk_text.push_str("-changed");
        assert_ne!(
            first,
            serde_json::to_vec(&fingerprint_payload(&changed_text)).expect("serialize"),
            "the chunk text is bound"
        );

        let mut changed_metadata = base.clone();
        changed_metadata.metadata_json = json!({"k": 2});
        assert_ne!(
            first,
            serde_json::to_vec(&fingerprint_payload(&changed_metadata)).expect("serialize"),
            "the payload metadata is bound"
        );

        let mut changed_hash = base;
        changed_hash.record_hash = "b".repeat(64);
        assert_ne!(
            first,
            serde_json::to_vec(&fingerprint_payload(&changed_hash)).expect("serialize"),
            "the record hash is bound"
        );
    }

    #[test]
    fn an_empty_replacement_request_is_refused_before_any_access() {
        let error = validate_request(&request(Vec::new()))
            .expect_err("an empty section list must be refused");
        assert!(error.to_string().contains("at least one section"));
    }

    #[test]
    fn a_repeated_section_key_is_refused_before_any_access() {
        let error = validate_request(&request(vec![section("a"), section("a")]))
            .expect_err("a repeated section key must be refused");
        assert!(error.to_string().contains("appears twice"));
    }

    #[test]
    fn a_blank_section_key_is_refused_before_any_access() {
        let error = validate_request(&request(vec![section("   ")]))
            .expect_err("a blank section key must be refused");
        assert!(error.to_string().contains("must not be empty"));
    }

    #[test]
    fn distinct_section_keys_pass_the_request_validation() {
        validate_request(&request(vec![section("a"), section("b")]))
            .expect("distinct section keys are accepted");
    }
}
