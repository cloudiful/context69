//! Task integration of the staged replacement executor (issue 1138 P17, P22).
//!
//! The fresh-source processor reads the current source bytes again and stages
//! the parsed sections through the P16 runner. This module owns the library
//! side of that integration: the one-snapshot replacement execution and the
//! atomic first insert that creates a file together with its item binding and
//! its durable run.

use anyhow::{Result, ensure};
use uuid::Uuid;

use crate::db::{Database, IngestRunMode, IngestRunSource, NewIngestRun};
use crate::domain::LibraryFileRecord;
use crate::library_store::{LibraryStore, NewLibraryFile, TaskWriteFence, begin_task_write};

use super::replacement_runner::{
    ReplacementRunOutcome, ReplacementRunRequest, ReplacementVector, stage_first_insert,
    stage_replacement,
};
use super::{LibraryService, TaskIngestStep, UnifiedIngestError};

/// The outcome of one atomic first insert.
pub enum AtomicFirstInsert {
    /// The file was created and bound to the running item with its durable
    /// first-insert run.
    Created { file_id: Uuid },
    /// The identity was taken by another writer while this one waited for the
    /// identity lock, so the caller replaces that file instead of creating a
    /// second one.
    Existing(LibraryFileRecord),
}

/// Binds the existing file to the running item and claims its durable
/// replacement run, leaving the freshly staged input attached.
///
/// `Ok(false)` means another task item holds an unresolved replacement run on
/// the same file. The caller rolls nothing back itself: the guarded write is
/// rolled back here before the refusal is reported, so no item or file state
/// changes.
pub(crate) async fn begin_ordinary_replacement(
    db: &Database,
    task_id: Uuid,
    item: &mut crate::db::ClaimedItem,
    file_id: Uuid,
    source: IngestRunSource,
) -> Result<bool> {
    let fence = TaskWriteFence::from_claimed_item(item)?;
    let mut write = begin_task_write(db, fence).await?;
    // The durable run of the item is read before the claim, so the first
    // adoption of the item into the atomic mode can be told from a retry.
    let existed = db.read_library_ingest_run_in(&mut write).await?.is_some();
    let run = db
        .begin_ordinary_replacement_run_in(&mut write, task_id, file_id, source)
        .await?;
    if run.is_none() {
        write.rollback().await;
        return Ok(false);
    }
    if !write.commit().await? {
        anyhow::bail!("the grant no longer authorizes the replacement claim");
    }
    item.file_id = Some(file_id);
    if !existed {
        clear_ingest_cache(db, item).await?;
    }
    Ok(true)
}

/// Drops the cached ingest state of one running item.
///
/// An item that enters the atomic mode must read its own current source, so
/// the sections a previous attempt parsed, the indexing checkpoint of that
/// parse, the pipeline progress and the sections artifact of the item are
/// dropped. No document, point, version or result is touched.
pub(crate) async fn clear_ingest_cache(db: &Database, item: &crate::db::ClaimedItem) -> Result<()> {
    let fence = TaskWriteFence::from_claimed_item(item)?;
    let mut connection = db.pool().acquire().await?;
    let cleared = db
        .clear_task_item_ingest_cache_in(
            &mut connection,
            item.id,
            fence.token(),
            fence.execution_task_id(),
            fence.generation(),
        )
        .await?;
    ensure!(
        cleared,
        "the task item lease was lost while clearing the cached ingest state"
    );
    Ok(())
}

/// Creates the file of a first insert and binds it to the running item.
///
/// One guarded transaction owns the whole binding: the sorted identity locks of
/// the group plus external id and the group plus content digest, the re-probe
/// of the identity, the ownership check of the staged source object, the
/// creation of the file and the claim of the durable first-insert run. A
/// created file is therefore never observable without the item and run that
/// own it, and a concurrent writer of the same identity waits for the lock and
/// then finds the committed file instead of creating a second one.
pub(crate) async fn begin_atomic_first_insert(
    db: &Database,
    store: &LibraryStore,
    task_id: Uuid,
    item: &mut crate::db::ClaimedItem,
    group_id: i64,
    staged_object: Uuid,
    file: &super::AtomicFirstInsertFile,
    metadata: Option<&crate::contracts::CanonicalUploadMetadata>,
    translation: Option<&crate::contracts::TranslationDirective>,
    source: IngestRunSource,
) -> Result<AtomicFirstInsert> {
    let fence = TaskWriteFence::from_claimed_item(item)?;
    let mut write = begin_task_write(db, fence).await?;

    let object = store
        .get_storage_object_by_id_on_connection(write.connection(), staged_object)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("the staged source object {staged_object} no longer exists")
        })?;
    ensure!(
        object.group_id == group_id,
        "the staged source object belongs to another group"
    );

    db.lock_library_identity_in(
        write.connection(),
        group_id,
        file.external_id.as_deref(),
        &object.sha256,
    )
    .await?;

    // The identity is re-probed under the lock: the writer that held the lock
    // before this one may have committed the file in the meantime.
    let existing = match file.external_id.as_deref() {
        Some(external_id) => {
            store
                .get_file_by_external_id_in_project_on(write.connection(), group_id, external_id)
                .await?
        }
        None => {
            store
                .get_file_by_sha_in_project_on(write.connection(), group_id, &object.sha256)
                .await?
        }
    };
    if let Some(existing) = existing {
        write.rollback().await;
        return Ok(AtomicFirstInsert::Existing(existing));
    }

    let new_file = NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: file.folder_id,
        external_id: file.external_id.clone(),
        filename: file.filename.clone(),
        media_type: file.media_type.clone(),
        size_bytes: object.size_bytes,
        sha256: object.sha256.clone(),
        storage_rel_path: object.object_key.clone(),
        storage_object_id: Some(staged_object),
        delete_source_after_processing: file.delete_source_after_processing,
    };

    let created = store
        .create_file_in_project_on(write.connection(), group_id, &new_file)
        .await?;
    if let Some(metadata) = metadata {
        store
            .update_business_metadata_in(write.connection(), group_id, created.id, metadata)
            .await?;
    }
    if let Some(translation) = translation {
        store
            .set_file_translation_directive_in(write.connection(), created.id, Some(translation))
            .await?;
    }

    let bound = db
        .set_task_item_file_keep_input_in(
            write.connection(),
            item.id,
            fence.token(),
            fence.execution_task_id(),
            fence.generation(),
            created.id,
        )
        .await?;
    ensure!(bound, "task item association or lease is no longer valid");

    db.claim_or_create_library_ingest_run_in(
        &mut write,
        task_id,
        created.id,
        Some(NewIngestRun {
            mode: IngestRunMode::FirstInsert,
            source,
        }),
    )
    .await?;

    if !write.commit().await? {
        anyhow::bail!("the grant no longer authorizes the first insert claim");
    }
    item.file_id = Some(created.id);
    Ok(AtomicFirstInsert::Created {
        file_id: created.id,
    })
}

impl LibraryService {
    /// The current file row of a fresh-source replacement.
    pub(crate) async fn file_for_task_source(
        &self,
        file_id: Uuid,
    ) -> Result<LibraryFileRecord, UnifiedIngestError> {
        self.task_file(file_id).await
    }

    /// The storage object that holds the current source of one file.
    ///
    /// The atomic first insert publishes the source it created, so the object
    /// that holds it is released from its staging lease once the switch
    /// committed.
    pub(crate) async fn file_source_object_id(&self, file_id: Uuid) -> Result<Option<Uuid>> {
        let paths = self.store.list_storage_paths_for_files(&[file_id]).await?;
        Ok(paths
            .iter()
            .find(|path| path.id == file_id)
            .and_then(|path| path.storage_object_id))
    }

    /// Creates the file of an atomic first insert and binds it to the running
    /// item with its durable first-insert run.
    ///
    /// The staged source object carries the content of the new file, so the
    /// identity of the file is decided from that object and never from bytes
    /// the caller still holds.
    pub async fn begin_atomic_first_insert_for_task(
        &self,
        db: &Database,
        task_id: Uuid,
        item: &mut crate::db::ClaimedItem,
        group_id: i64,
        staged_object: Uuid,
        file: &super::AtomicFirstInsertFile,
        metadata: Option<&crate::contracts::CanonicalUploadMetadata>,
        translation: Option<&crate::contracts::TranslationDirective>,
        source: IngestRunSource,
    ) -> Result<AtomicFirstInsert> {
        // The folder of the new file must belong to the group, exactly like in
        // the ordinary upload path.
        if let Some(folder_id) = file.folder_id
            && self
                .store
                .get_folder_in_project(group_id, folder_id)
                .await?
                .is_none()
        {
            anyhow::bail!("unknown folder {folder_id}");
        }
        begin_atomic_first_insert(
            db,
            &self.store,
            task_id,
            item,
            group_id,
            staged_object,
            file,
            metadata,
            translation,
            source,
        )
        .await
    }

    /// One bounded staging turn of a replacement task.
    ///
    /// The embedding provider and the index are captured together, so no
    /// provider from one generation is combined with an index from another. No
    /// connection is held across the embedding or Qdrant calls.
    pub(crate) async fn stage_task_replacement(
        &self,
        db: &Database,
        fence: TaskWriteFence,
        request: &ReplacementRunRequest,
    ) -> Result<TaskIngestStep<ReplacementRunOutcome>> {
        let batches_per_segment = self.ingest_batches_per_segment().await?;
        let (runtime, identity) = self.captured_runtime()?;
        let embedding = runtime.embedding.require_ready()?;
        let vector = ReplacementVector {
            embedding: embedding.as_ref(),
            index: &runtime.index,
            chunking: &self.chunking,
            identity,
        };
        stage_replacement(
            db,
            &self.store,
            &vector,
            fence,
            request,
            batches_per_segment,
        )
        .await
    }

    /// One bounded staging turn of an atomic first insert.
    pub(crate) async fn stage_task_first_insert(
        &self,
        db: &Database,
        fence: TaskWriteFence,
        request: &ReplacementRunRequest,
    ) -> Result<TaskIngestStep<ReplacementRunOutcome>> {
        let batches_per_segment = self.ingest_batches_per_segment().await?;
        let (runtime, identity) = self.captured_runtime()?;
        let embedding = runtime.embedding.require_ready()?;
        let vector = ReplacementVector {
            embedding: embedding.as_ref(),
            index: &runtime.index,
            chunking: &self.chunking,
            identity,
        };
        stage_first_insert(
            db,
            &self.store,
            &vector,
            fence,
            request,
            batches_per_segment,
        )
        .await
    }
}
