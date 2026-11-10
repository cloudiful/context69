//! The atomic library replacement switch kernel (issue #1138 P15).
//!
//! One fenced transaction moves a library file from its current content to a
//! freshly prepared replacement. The documents and their versions, the chunks
//! of the affected sections, the file links, the file source and status, the
//! search registry entry and the durable ingest run all move together, or none
//! of them does.
//!
//! The kernel owns no external call. Every statement runs on the connection of
//! the guarded transaction of the task item, so a lost grant rolls back every
//! layer and the caller's own commit check refuses a write whose lease expired
//! while it waited.
//!
//! The caller owns the two steps that precede the switch: it must have
//! committed the registry admission of the complete staged point set, and it
//! must have verified that every prepared point exists. Nothing here proves
//! either, and the kernel never writes a vector point itself; the shared
//! replacement runner and the ordinary library ingest processors are its
//! production callers.

use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::db::{Database, IngestRunMode, IngestRunSource, NewIngestRun};
use crate::domain::{ChunkPayload, DocumentChunk, LibraryFileDocumentRecord};
use crate::library_store::{LibraryStore, TaskWriteFence, begin_task_write};

use super::ReplacementFileAttributes;

/// The baseline of the library file a replacement replaces.
///
/// The fields mirror the locked file row: the kernel rebuilds the baseline
/// from the row it locks and refuses the switch when the rebuilt baseline
/// differs from the one the replacement was prepared against. The row version
/// (`updated_at`) is part of the baseline, so a file that changed in any way
/// after the preparation refuses the switch instead of overwriting newer data.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementFileBaseline {
    pub group_id: i64,
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub source_uri: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub metadata_json: serde_json::Value,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
    pub source_released_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// The baseline of one affected document.
///
/// A document that the replacement overwrites keeps its identity and its
/// history, so the kernel compares the stored record hash with the one the
/// replacement was prepared against and refuses a switch that would overwrite
/// a newer baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementDocumentBaseline {
    pub document_id: i64,
    pub record_hash: String,
}

/// One freshly normalized section of a prepared replacement.
#[derive(Debug, Clone)]
pub struct PreparedReplacementSection {
    /// The final document id of the section.
    ///
    /// An existing section keeps the document id it already has. A new section
    /// carries the id that was reserved from the document sequence before the
    /// replacement was staged, so the final id is known before the points are
    /// written.
    pub document_id: i64,
    /// The document payload of the section.
    pub payload: ChunkPayload,
    /// The generation chunks of the section.
    pub chunks: Vec<DocumentChunk>,
    /// The file link of the section.
    pub link: LibraryFileDocumentRecord,
}

/// The prepared replacement of one library file.
#[derive(Debug, Clone)]
pub struct PreparedReplacement {
    /// The durable batch id of the ingest run, which is also the run id of the
    /// search registry entry.
    pub run_id: Uuid,
    /// The file the replacement belongs to.
    pub file_id: Uuid,
    /// The mode the durable run was claimed with.
    ///
    /// The kernel refuses a run whose stored mode differs, so a first insert
    /// can never be applied as a replacement and a replacement can never be
    /// applied as a first insert.
    pub mode: IngestRunMode,
    /// The source disposition of the durable run.
    pub source: IngestRunSource,
    /// The baseline the replacement was prepared against.
    pub baseline: ReplacementFileBaseline,
    /// The baseline of every document the replacement overwrites.
    ///
    /// A section whose document is not listed here is a new section: the
    /// kernel inserts it under its reserved id and retires nothing for it.
    pub baseline_documents: Vec<ReplacementDocumentBaseline>,
    /// The freshly normalized sections of the replacement.
    pub sections: Vec<PreparedReplacementSection>,
    /// The projected file attributes the ordinary update publishes atomically
    /// with the switch. `None` keeps the current row attributes.
    pub attributes: Option<ReplacementFileAttributes>,
    /// The staged source object attached to the task item.
    ///
    /// The kernel promotes it only when the same task item references it and
    /// the object belongs to the group of the file.
    pub staged_source_object: Option<Uuid>,
}

/// How a switch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementSwitchDisposition {
    /// The replacement was applied by this call.
    Switched,
    /// The durable run was already resolved by an earlier switch, so this
    /// call changed nothing.
    AlreadyApplied,
}

/// The observable result of one switch.
#[derive(Debug, Clone)]
pub struct ReplacementSwitchOutcome {
    pub disposition: ReplacementSwitchDisposition,
    /// The documents of the replacement, in section order.
    pub document_ids: Vec<i64>,
    /// The point ids of the version that is no longer live.
    pub retired_point_ids: Vec<Uuid>,
}

/// Applies one prepared replacement to its library file.
///
/// The switch runs inside the guarded transaction of the task item: the item
/// row is locked first, the registry epoch second, and the file and document
/// rows last, so every fenced switch takes the same lock order.
///
/// A run that was already resolved by an earlier switch reports
/// [`ReplacementSwitchDisposition::AlreadyApplied`] and writes nothing, so a
/// replay never reverts a state that moved on after the first switch. A run
/// whose registry entry was retired was abandoned and never counts as a
/// success.
pub async fn switch_replacement(
    db: &Database,
    store: &LibraryStore,
    fence: TaskWriteFence,
    plan: &PreparedReplacement,
) -> Result<ReplacementSwitchOutcome> {
    validate_plan(plan)?;
    let mut write = begin_task_write(db, fence).await?;

    // The registry epoch is locked before any file or document row.
    db.lock_search_registry_epoch_in(write.connection()).await?;
    // A rebuild owns the whole collection while it runs, so a switch that
    // overlapped it would publish points the rebuild overwrites or loses.
    db.require_rebuild_idle_in(write.connection()).await?;
    let registry = db
        .read_search_registry_entry_in(write.connection(), plan.run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the replacement run has no registry entry"))?;

    // The identity the run was admitted under is compared with the persisted
    // configuration inside the switch transaction. The persisted settings are
    // the authority: a node whose own snapshot predates a settings change on
    // another node must not make its points visible.
    db.require_matching_vector_identity_in(write.connection(), plan.run_id)
        .await?;

    let run = db
        .read_library_ingest_run_in(&mut write)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the replacement run is not registered"))?;
    ensure!(
        run.item_id == fence.item_id(),
        "the replacement run belongs to another task item"
    );
    ensure!(
        run.file_id == plan.file_id,
        "the replacement run belongs to another library file"
    );
    ensure!(
        run.batch_id == plan.run_id,
        "the replacement run is another durable run"
    );
    ensure!(
        run.mode == plan.mode,
        "the durable run has another mode than the prepared one"
    );
    ensure!(
        run.source == plan.source,
        "the replacement run has another source disposition"
    );

    match (run.resolved_at.is_some(), registry.phase.as_str()) {
        (true, "switched") | (true, "retired") => {
            write.rollback().await;
            return Ok(ReplacementSwitchOutcome {
                disposition: ReplacementSwitchDisposition::AlreadyApplied,
                document_ids: plan
                    .sections
                    .iter()
                    .map(|section| section.document_id)
                    .collect(),
                retired_point_ids: Vec::new(),
            });
        }
        (true, phase) => {
            bail!("the resolved replacement run has no switched registry entry ({phase})")
        }
        (false, "retired") => {
            bail!("the replacement run was abandoned and never counts as a success")
        }
        (false, "staging") => {}
        (false, phase) => bail!("the replacement registry entry is in the phase {phase}"),
    }

    ensure!(
        registry.file_id == Some(plan.file_id),
        "the replacement registry entry belongs to another library file"
    );
    ensure!(
        registry.generation == run.generation,
        "the replacement registry entry has another generation"
    );
    let intended: Vec<Uuid> = plan
        .sections
        .iter()
        .flat_map(|section| section.chunks.iter().map(|chunk| chunk.id))
        .collect();
    ensure!(
        is_same_id_set(&registry.staged_ids, &intended),
        "the registered staged set does not match the prepared chunks"
    );

    // A lease handover leaves the durable run on the previous grant. The run
    // identity is immutable, so the current lease only adopts the lease
    // fields of the existing row.
    let run = if run.lease_token != fence.token()
        || run.execution_task_id != fence.execution_task_id()
        || run.generation != fence.generation()
    {
        db.claim_or_create_library_ingest_run_in(
            &mut write,
            run.task_id,
            plan.file_id,
            Some(NewIngestRun {
                mode: plan.mode,
                source: plan.source,
            }),
        )
        .await?
    } else {
        run
    };

    let locked_file = store
        .lock_replacement_file_on(write.connection(), plan.file_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the library file no longer exists"))?;
    let current_baseline = ReplacementFileBaseline {
        group_id: locked_file.group_id,
        folder_id: locked_file.folder_id,
        external_id: locked_file.external_id.clone(),
        source_uri: locked_file.source_uri.clone(),
        published_at: locked_file.published_at,
        metadata_json: locked_file.metadata_json.clone(),
        filename: locked_file.filename.clone(),
        media_type: locked_file.media_type.clone(),
        size_bytes: locked_file.size_bytes,
        sha256: locked_file.sha256.clone(),
        storage_rel_path: locked_file.storage_rel_path.clone(),
        storage_object_id: locked_file.storage_object_id,
        source_released_at: locked_file.source_released_at,
        updated_at: locked_file.updated_at,
    };
    ensure!(
        current_baseline == plan.baseline,
        "the library file changed since the replacement was prepared"
    );

    if plan.mode == IngestRunMode::FirstInsert {
        // A first insert publishes a file that never carried committed
        // content: no linked document, no committed version behind it, and a
        // status that is not the succeeded one. A file that already carries
        // any of them belongs to the replacement path, which preserves its
        // documents, versions and results instead of dropping them.
        let record = store
            .lock_file_on(write.connection(), plan.file_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("the library file no longer exists"))?;
        ensure!(
            record.ingest_status != crate::contracts::LibraryIngestStatus::Succeeded,
            "the first insert file is already ingested"
        );
        ensure!(
            record.ingested_at.is_none(),
            "the first insert file already carries an ingest timestamp"
        );
        let linked = store
            .list_file_documents_on(write.connection(), plan.file_id)
            .await?;
        ensure!(
            linked.is_empty(),
            "the first insert file already links {} documents",
            linked.len()
        );
    }

    let mut affected_ids: Vec<i64> = plan
        .baseline_documents
        .iter()
        .map(|baseline| baseline.document_id)
        .collect();
    affected_ids.extend(plan.sections.iter().map(|section| section.document_id));
    affected_ids.sort_unstable();
    affected_ids.dedup();
    let locked_documents = db
        .lock_replacement_documents_on(write.connection(), &affected_ids)
        .await?;
    for baseline in &plan.baseline_documents {
        let locked = locked_documents
            .iter()
            .find(|document| document.id == baseline.document_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "the affected document {} no longer exists",
                    baseline.document_id
                )
            })?;
        ensure!(
            locked.record_hash == baseline.record_hash,
            "the affected document {} changed since the replacement was prepared",
            baseline.document_id
        );
    }
    for section in &plan.sections {
        let existed = locked_documents
            .iter()
            .any(|document| document.id == section.document_id);
        let baselined = plan
            .baseline_documents
            .iter()
            .any(|baseline| baseline.document_id == section.document_id);
        ensure!(
            existed == baselined,
            "the section document {} does not match its baseline",
            section.document_id
        );
    }

    // The retired set is the point set of the affected documents as it is
    // stored right now, never a set the caller supplies. A stored translation
    // joins it when its version was translated from a source hash the
    // replacement does not publish, so every translation that becomes
    // unhydratable leaves the search results with the switch. A translation
    // of the new hash survives a same-hash reindex.
    //
    // The original subset is read before the canonical rows are deleted: only
    // that producer query can prove which retired ids were old originals. The
    // translation subset is not provable afterwards, so it is never recorded
    // as an original and stays fully reserved.
    let mut retired_point_ids: Vec<Uuid> = Vec::new();
    let mut retired_original_ids: Vec<Uuid> = Vec::new();
    for baseline in &plan.baseline_documents {
        let new_hash = plan
            .sections
            .iter()
            .find(|section| section.document_id == baseline.document_id)
            .map(|section| section.payload.record_hash.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "the affected document {} has no prepared section",
                    baseline.document_id
                )
            })?;
        let originals = db
            .replacement_point_ids_on(write.connection(), baseline.document_id, new_hash, false)
            .await?;
        retired_original_ids.extend(originals);
        let ids = db
            .replacement_point_ids_on(write.connection(), baseline.document_id, new_hash, true)
            .await?;
        retired_point_ids.extend(ids);
    }
    let retired_point_ids = dedup_ids(retired_point_ids);
    let retired_original_ids = dedup_ids(retired_original_ids);
    let retired_set: BTreeSet<Uuid> = retired_point_ids.iter().copied().collect();
    ensure!(
        retired_original_ids
            .iter()
            .all(|id| retired_set.contains(id)),
        "the retired original ids are not a subset of the retired ids"
    );

    let mut document_ids: Vec<i64> = Vec::new();
    for section in &plan.sections {
        let upserted = db
            .upsert_document_with_reserved_id_in(
                write.connection(),
                &section.payload,
                section.document_id,
            )
            .await?;
        ensure!(
            upserted.document_id == section.document_id,
            "the section document identity changed"
        );
        db.delete_document_chunks_in(write.connection(), section.document_id)
            .await?;
        db.insert_document_chunks_in(
            write.connection(),
            section.document_id,
            &section.payload.record_hash,
            &section.chunks,
        )
        .await?;
        document_ids.push(section.document_id);
    }

    let links: Vec<LibraryFileDocumentRecord> = plan
        .sections
        .iter()
        .map(|section| section.link.clone())
        .collect();
    store
        .merge_file_documents_on(write.connection(), plan.file_id, &links)
        .await?;

    let switched = store
        .switch_replacement_file_in(
            &mut write,
            &locked_file,
            plan.staged_source_object,
            plan.attributes.as_ref(),
        )
        .await?;
    ensure!(
        switched,
        "the library file changed since the replacement was prepared"
    );

    // The old source is released on the same transaction, and only when it is
    // a different object that no other row references any more.
    if let Some(old_object) = locked_file.storage_object_id {
        if Some(old_object) != plan.staged_source_object {
            let references = store
                .count_storage_object_references_on_connection(write.connection(), old_object)
                .await?;
            if references == 0 {
                if let Some(object) = store
                    .get_storage_object_by_id_on_connection(write.connection(), old_object)
                    .await?
                {
                    store
                        .insert_source_object_cleanup(
                            write.connection(),
                            Some(old_object),
                            locked_file.group_id,
                            Some(&object.sha256),
                            &object.object_key,
                            &object.storage_backend,
                        )
                        .await?;
                }
            }
        }
    }

    let switched_epoch = db
        .switch_search_registry_entry_with_retired_origin_in(
            plan.run_id,
            &retired_point_ids,
            &retired_original_ids,
            write.connection(),
        )
        .await?;
    ensure!(
        switched_epoch.is_some(),
        "the replacement registry entry did not switch"
    );

    db.bump_search_generation_in(write.connection()).await?;
    let resolved = db
        .mark_library_ingest_run_resolved_in(&mut write, run.task_id, plan.file_id)
        .await?;
    ensure!(resolved, "the replacement run was not resolved");

    let committed = write.commit().await?;
    ensure!(
        committed,
        "the grant no longer authorizes the replacement switch"
    );

    Ok(ReplacementSwitchOutcome {
        disposition: ReplacementSwitchDisposition::Switched,
        document_ids,
        retired_point_ids,
    })
}

/// The internal consistency of one prepared replacement.
fn validate_plan(plan: &PreparedReplacement) -> Result<()> {
    let mut section_keys: BTreeSet<&str> = BTreeSet::new();
    let mut document_ids: BTreeSet<i64> = BTreeSet::new();
    for section in &plan.sections {
        ensure!(
            section_keys.insert(section.link.section_key.as_str()),
            "the section key {} appears twice",
            section.link.section_key
        );
        ensure!(
            document_ids.insert(section.document_id),
            "the document {} appears twice",
            section.document_id
        );
        ensure!(
            section.link.file_id == plan.file_id,
            "a section link belongs to another library file"
        );
        ensure!(
            section.link.document_id == section.document_id,
            "a section link belongs to another document"
        );
        ensure!(
            section
                .chunks
                .iter()
                .all(|chunk| chunk.document_id == section.document_id),
            "a chunk belongs to another document"
        );
        ensure!(
            section.payload.group_id == plan.baseline.group_id,
            "a section payload belongs to another group"
        );
    }
    for baseline in &plan.baseline_documents {
        ensure!(
            document_ids.contains(&baseline.document_id),
            "the baseline document {} is not a section of the replacement",
            baseline.document_id
        );
    }
    if plan.mode == IngestRunMode::FirstInsert {
        ensure!(
            plan.baseline_documents.is_empty(),
            "a first insert must not overwrite an existing document"
        );
        ensure!(
            plan.attributes.is_none(),
            "a first insert publishes the attributes of the created file"
        );
        ensure!(
            plan.staged_source_object.is_none(),
            "a first insert reads the source of the created file and promotes nothing"
        );
    }
    Ok(())
}

/// Whether two id lists hold the same ids, each of them without a duplicate.
fn is_same_id_set(left: &[Uuid], right: &[Uuid]) -> bool {
    let left_set: BTreeSet<Uuid> = left.iter().copied().collect();
    let right_set: BTreeSet<Uuid> = right.iter().copied().collect();
    left_set.len() == left.len() && right_set.len() == right.len() && left_set == right_set
}

/// The ids of a list without a duplicate, in their original order.
fn dedup_ids(ids: Vec<Uuid>) -> Vec<Uuid> {
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        PreparedReplacement, PreparedReplacementSection, ReplacementFileBaseline, dedup_ids,
        is_same_id_set, validate_plan,
    };
    use crate::db::IngestRunSource;
    use crate::domain::{ChunkPayload, LibraryFileDocumentRecord};
    use context69_contracts::Visibility;
    use uuid::Uuid;

    fn baseline() -> ReplacementFileBaseline {
        ReplacementFileBaseline {
            group_id: 1,
            folder_id: None,
            external_id: None,
            source_uri: None,
            published_at: None,
            metadata_json: serde_json::json!({}),
            filename: "file.txt".into(),
            media_type: "text/plain".into(),
            size_bytes: 4,
            sha256: "0".repeat(64),
            storage_rel_path: "objects/old".into(),
            storage_object_id: None,
            source_released_at: None,
            updated_at: chrono::Utc::now(),
        }
    }

    fn link(file_id: Uuid, document_id: i64, key: &str) -> LibraryFileDocumentRecord {
        LibraryFileDocumentRecord {
            file_id,
            document_id,
            group_id: 1,
            visibility: Visibility::Private,
            section_key: key.into(),
            section_label: key.into(),
            section_external_id: None,
            section_source_uri: None,
            section_published_at: None,
            section_metadata_json: serde_json::json!({}),
            sort_order: 0,
        }
    }

    fn section(file_id: Uuid, document_id: i64, key: &str) -> PreparedReplacementSection {
        PreparedReplacementSection {
            document_id,
            payload: ChunkPayload {
                chunk_id: Uuid::nil(),
                document_id,
                group_id: 1,
                group_key: "key".into(),
                group_path: "path".into(),
                visibility: Visibility::Private,
                source_key: "library".into(),
                external_id: format!("external-{document_id}"),
                title: "title".into(),
                summary: None,
                source_uri: "context69://test".into(),
                published_at: None,
                updated_at_source: chrono::Utc::now(),
                record_hash: "hash".into(),
                chunk_index: 0,
                chunk_text: "text".into(),
                metadata_json: serde_json::json!({}),
                content_locale: "original".into(),
                source_locale: None,
                translation_provider: None,
            },
            chunks: Vec::new(),
            link: link(file_id, document_id, key),
        }
    }

    fn plan(file_id: Uuid, sections: Vec<PreparedReplacementSection>) -> PreparedReplacement {
        PreparedReplacement {
            run_id: Uuid::new_v4(),
            file_id,
            mode: crate::db::IngestRunMode::Replacement,
            source: IngestRunSource::Stored,
            baseline: baseline(),
            baseline_documents: Vec::new(),
            sections,
            attributes: None,
            staged_source_object: None,
        }
    }

    #[test]
    fn the_same_id_set_needs_the_same_ids_without_a_duplicate() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        assert!(is_same_id_set(&[first, second], &[second, first]));
        assert!(!is_same_id_set(&[first, second], &[first]));
        assert!(!is_same_id_set(&[first, first], &[first, second]));
        assert!(is_same_id_set(&[], &[]));
    }

    #[test]
    fn the_deduplicated_ids_keep_their_order() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        assert_eq!(
            dedup_ids(vec![first, second, first, second]),
            vec![first, second]
        );
    }

    #[test]
    fn a_plan_rejects_a_repeated_section_key_and_a_foreign_link() {
        let file = Uuid::new_v4();
        let repeated = plan(
            file,
            vec![section(file, 1, "section"), section(file, 2, "section")],
        );
        assert!(validate_plan(&repeated).is_err());

        let foreign = plan(file, vec![section(Uuid::new_v4(), 1, "section")]);
        assert!(validate_plan(&foreign).is_err());

        let repeated_document = plan(file, vec![section(file, 1, "one"), section(file, 1, "two")]);
        assert!(validate_plan(&repeated_document).is_err());
    }

    #[test]
    fn a_baseline_document_is_a_section_of_the_replacement() {
        let file = Uuid::new_v4();
        let mut prepared = plan(file, vec![section(file, 1, "section")]);
        prepared.baseline_documents = vec![super::ReplacementDocumentBaseline {
            document_id: 2,
            record_hash: "hash".into(),
        }];
        assert!(validate_plan(&prepared).is_err());
    }
}
