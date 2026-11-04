//! The fresh-source replacement processor (issue 1138 P17).
//!
//! One succeeded file/url/text parent is forked into a fresh task item by
//! [`crate::db::Database::rerun_task_source`]. This processor reads the current
//! source again, parses it into fresh sections, and applies the P16 staged
//! replacement runner. It is the only caller of the runner.
//!
//! The processor stays inactive until the registry capacity policy is
//! configured: a service without one refuses the item terminally instead of
//! inventing a budget. Transient embed, upsert or
//! switch failures keep the fresh source, the sections and the committed
//! registry staging in place for a retry; no destructive cleanup runs here.

use context69_contracts::{ImportLibraryFileFromUrlRequest, IngestOptions, TaskKind};
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{
    ProcessResult, process_error, resolved_section_payload, set_file, waiting_for_error,
};
use crate::db::{ClaimedItem, IngestRunMode, IngestRunSource};
use crate::library_store::TaskWriteFence;
use crate::services::library::{
    AtomicFirstInsert, AtomicFirstInsertFile, IngestSection, ReplacementRunRequest,
    ReplacementSectionInput, TaskIngestStep, TaskSourceRef,
};

/// Refuses an atomic segment whose captured staged pointer is missing or no
/// longer the persisted input of its captured grant.
///
/// A run-owning item may only read the object its exact execution persisted.
/// A missing pointer must never fall back to the canonical file, and a stale,
/// same-group-other or foreign pointer must never reach a remote intent or a
/// canonical write. The check runs before every atomic source read.
async fn require_persisted_atomic_input(
    service: &TaskService,
    item: &ClaimedItem,
) -> anyhow::Result<Option<ProcessResult>> {
    let Some(execution) = item.execution_context() else {
        return Ok(Some(terminal(
            "storage",
            "task item has no captured execution",
        )));
    };
    let persisted = service
        .db()
        .get_item_input_storage_object(item.id, &execution)
        .await?;
    match (item.input_storage_object_id, persisted) {
        (Some(captured), Some(persisted)) if captured == persisted => Ok(None),
        _ => Ok(Some(terminal(
            "storage",
            "the atomic source is missing or does not match the persisted item grant",
        ))),
    }
}

/// The atomic branch of one ordinary task item.
///
/// The durable run is the authority of the mode: a replacement run replaces
/// the content of the file it is bound to, a first-insert run publishes the
/// file it created. The two never share a path, so a first insert can not be
/// applied as a replacement.
pub(super) async fn process_atomic_ingest(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    mode: IngestRunMode,
) -> anyhow::Result<ProcessResult> {
    if let Some(refusal) = require_persisted_atomic_input(service, item).await? {
        return Ok(refusal);
    }
    match mode {
        IngestRunMode::Replacement => {
            process_ordinary_replacement(service, group, task, item).await
        }
        IngestRunMode::FirstInsert => process_atomic_first_insert(service, group, task, item).await,
    }
}

/// The staged source object of one first insert.
///
/// The bytes of the task are staged before the transaction, so the file that
/// the transaction creates already points at the object that holds them. The
/// captured item carries that object as its physical source, so the resumed
/// atomic segment binds its identity to the same staged object instead of
/// falling back to the canonical row. An item that already carries a staged
/// object keeps it: the bytes were staged from the same source.
pub(super) async fn stage_first_insert_source(
    service: &TaskService,
    group_id: i64,
    item: &mut ClaimedItem,
    upload: &crate::services::library::UploadedLibraryFile,
) -> anyhow::Result<Uuid> {
    if let Some(object_id) = item.input_storage_object_id {
        return Ok(object_id);
    }
    let staged = service
        .library()
        .stage_file_for_task_input(group_id, upload.clone())
        .await?;
    let execution = item.execution_context().ok_or_else(|| {
        crate::domain_errors::DomainError::conflict("task item has no captured execution")
    })?;
    if !service
        .db()
        .set_task_item_input_storage_object(item.id, &execution, staged, &item.payload)
        .await?
    {
        let _ = service
            .library()
            .release_task_input_staging(staged, None)
            .await;
        anyhow::bail!(crate::domain_errors::DomainError::conflict(
            "task item lease was lost while saving staged first-insert input"
        ));
    }
    item.input_storage_object_id = Some(staged);
    Ok(staged)
}

/// Claims the atomic first insert of one ordinary task item.
///
/// The source is staged before the transaction, and the file, the item binding
/// and the durable run are created inside it. A file that another writer took
/// in the meantime is never replaced here: the caller waits for that writer
/// instead of changing any pointer.
pub(super) async fn claim_atomic_first_insert(
    service: &TaskService,
    group_id: i64,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    file: &AtomicFirstInsertFile,
    staged_object: Uuid,
    metadata: Option<&context69_contracts::CanonicalUploadMetadata>,
    translation: Option<&context69_contracts::TranslationDirective>,
    source: IngestRunSource,
) -> anyhow::Result<ProcessResult> {
    let claimed = service
        .library()
        .begin_atomic_first_insert_for_task(
            service.db(),
            task.id,
            item,
            group_id,
            staged_object,
            file,
            metadata,
            translation,
            source,
        )
        .await;
    match claimed {
        Ok(AtomicFirstInsert::Created { file_id }) => {
            anyhow::ensure!(
                item.file_id == Some(file_id),
                "the created file is not bound to the running item"
            );
            crate::services::library::clear_ingest_cache(service.db(), item).await?;
            let next = match service
                .library()
                .file_ingest_stage(&file.filename, &file.media_type)
            {
                Ok(next) => next,
                Err(error) => return Ok(process_error("storage", error)),
            };
            Ok(ProcessResult::Yielded { next })
        }
        Ok(AtomicFirstInsert::Existing(_existing)) => Ok(waiting_for_error(
            item,
            crate::services::library::UnifiedIngestError {
                stage: "storage".to_string(),
                dependency_key: None,
                retryable: true,
                message: "another task item already created this library file".to_string(),
            },
        )),
        Err(error) => Ok(process_error("storage", error)),
    }
}

/// Publishes the file of one atomic first insert.
///
/// The file was created together with the durable run, so it never carried
/// committed content: the fresh sections are parsed from the current source of
/// that file, and the same runner that applies a replacement stages their
/// vectors and switches the documents, chunks, links, status, registry entry
/// and run together.
pub(super) async fn process_atomic_first_insert(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
) -> anyhow::Result<ProcessResult> {
    let Some(quota) = service.replacement_caps() else {
        return Ok(terminal(
            "first_insert",
            "the atomic first insert is disabled until the registry capacity policy is configured",
        ));
    };
    let Some(group) = group else {
        return Ok(terminal(
            "first_insert",
            "the atomic first insert requires a group",
        ));
    };
    let Some(file_id) = item.file_id else {
        return Ok(terminal(
            "first_insert",
            "the first insert item has no library file",
        ));
    };
    let file = match service.library().file_for_task_source(file_id).await {
        Ok(file) => file,
        Err(error) => return Ok(ingest_failure_result(item, error)),
    };
    if file.group_id != group.id {
        return Ok(terminal(
            "first_insert",
            "the first insert file belongs to another group",
        ));
    }
    let kind = super::responses::parse_kind(&task.kind)?;
    let source = match kind {
        TaskKind::UrlBatch => IngestRunSource::Url,
        TaskKind::FileBatch | TaskKind::TextBatch => IngestRunSource::Stored,
        other => {
            return Ok(terminal(
                "first_insert",
                format!("the atomic first insert does not support the {other:?} task kind"),
            ));
        }
    };
    let stored_kind =
        match super::super::library::storage::detect_file_kind(&file.filename, &file.media_type) {
            Ok(kind) => kind,
            Err(error) => return Ok(terminal("parsing", error.to_string())),
        };
    let sections = if matches!(
        stored_kind,
        crate::services::library::LibraryFileKind::Pdf
            | crate::services::library::LibraryFileKind::Docx
            | crate::services::library::LibraryFileKind::Xlsx
    ) {
        match resolved_sections(service, task, item, file_id).await? {
            Ok(sections) => sections,
            Err(result) => return Ok(result),
        }
    } else {
        match service
            .library()
            .prepare_task_source_sections(
                file_id,
                TaskSourceRef::from_input(item.input_storage_object_id),
                item.lease_token,
            )
            .await
        {
            Ok(sections) => sections,
            Err(error) => return Ok(ingest_failure_result(item, error)),
        }
    };
    let replacement_sections = sections
        .into_iter()
        .enumerate()
        .map(|(index, section)| ReplacementSectionInput {
            section_key: section.section_key,
            section_label: section.section_label,
            sort_order: index as i32,
            section_metadata_json: section.metadata_json,
            title: section.title,
            summary: section.summary,
            body_text: section.body_text,
            source_uri: section.source_uri,
            external_id: section.external_id,
            published_at: section.published_at,
        })
        .collect::<Vec<_>>();
    let request = ReplacementRunRequest {
        task_id: item.task_id,
        file_id,
        source,
        staged_source_object: None,
        quota,
        attributes: None,
        sections: replacement_sections,
    };
    let fence = TaskWriteFence::from_claimed_item(item)?;
    match service
        .library()
        .stage_task_first_insert(service.db(), fence, &request)
        .await
    {
        Ok(TaskIngestStep::Complete(_outcome)) => {
            set_file(service.db(), task.id, item, file_id).await?;
            item.input_storage_object_id = None;
            // The staged object is the live source of the created file now, so
            // its staging lease ends with the reference guards of the file.
            if let Some(object_id) = service.library().file_source_object_id(file_id).await? {
                service
                    .library()
                    .release_task_input_staging(object_id, Some(file_id))
                    .await?;
            }
            Ok(ProcessResult::Succeeded(Some(file_id.to_string())))
        }
        Ok(TaskIngestStep::Yielded) => Ok(ProcessResult::Yielded { next: "indexing" }),
        Err(error) => Ok(process_error("first_insert", error)),
    }
}

/// Completes an ordinary single-file update whose storage stage already
/// claimed a durable replacement run.
///
/// The branch ignores the resumed stage name: the durable run marker is the
/// authority, so a retry re-enters here even when the collapsed `storage`
/// entry stage would otherwise run again. Fresh sections are parsed from the
/// attached staged source under the projected file view, then handed to the
/// P16 runner.
pub(super) async fn process_ordinary_replacement(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
) -> anyhow::Result<ProcessResult> {
    let Some(quota) = service.replacement_caps() else {
        return Ok(terminal(
            "replacement",
            "ordinary replacement is disabled until the registry capacity policy is configured",
        ));
    };
    let Some(group) = group else {
        return Ok(terminal("replacement", "replacement requires a group"));
    };
    let Some(file_id) = item.file_id else {
        return Ok(terminal(
            "replacement",
            "replacement item has no library file",
        ));
    };
    let file = match service.library().file_for_task_source(file_id).await {
        Ok(file) => file,
        Err(error) => return Ok(ingest_failure_result(item, error)),
    };
    if file.group_id != group.id {
        return Ok(terminal(
            "replacement",
            "replacement file belongs to another group",
        ));
    }
    let kind = super::responses::parse_kind(&task.kind)?;
    let (attributes, run_source) = match kind {
        TaskKind::TextBatch => {
            let request: context69_contracts::UpsertLibraryTextRequest =
                match super::pipeline_progress_input::deserialize_worker_input(&item.payload) {
                    Ok(request) => request,
                    Err(error) => return Ok(process_error("replacement", error.into())),
                };
            match service
                .library()
                .text_replacement_candidate(group, &request)
                .await
            {
                Ok(Some((_, attributes, _))) => (attributes, IngestRunSource::Stored),
                Ok(None) => {
                    return Ok(terminal(
                        "replacement",
                        "the text file is no longer a replacement target",
                    ));
                }
                Err(error) => return Ok(process_error("replacement", error)),
            }
        }
        TaskKind::FileBatch => {
            let request: super::item_file_processors::StoredFileBatchItem =
                match super::pipeline_progress_input::deserialize_worker_input(&item.payload) {
                    Ok(request) => request,
                    Err(error) => return Ok(process_error("replacement", error.into())),
                };
            let upload = crate::services::library::UploadedLibraryFile {
                folder_id: request.folder_id,
                filename: request.filename,
                media_type: request.media_type,
                bytes: bytes::Bytes::new(),
                declared_sha256: None,
                options: request.options,
                staged_storage_object_id: item.input_storage_object_id,
            };
            (
                service
                    .library()
                    .projected_upload_attributes(&file, &upload),
                IngestRunSource::Stored,
            )
        }
        TaskKind::UrlBatch => {
            let request: context69_contracts::ImportLibraryFileFromUrlRequest =
                match super::item_url_processor::url_storage_request(&item.payload) {
                    Ok(request) => request,
                    Err(error) => return Ok(process_error("replacement", error)),
                };
            let artifact = match super::item_url_processor::downloaded_artifact(&item.payload) {
                Some(artifact) => artifact,
                None => {
                    return Ok(terminal(
                        "replacement",
                        "URL replacement is missing its downloaded artifact",
                    ));
                }
            };
            let mut ingest = request.options;
            if ingest.metadata.source_uri.is_none() {
                ingest.metadata.source_uri = Some(artifact.source_url);
            }
            let upload = crate::services::library::UploadedLibraryFile {
                folder_id: request.folder_id,
                filename: artifact.filename,
                media_type: artifact.media_type,
                bytes: bytes::Bytes::new(),
                declared_sha256: Some(artifact.sha256),
                options: ingest,
                staged_storage_object_id: item.input_storage_object_id,
            };
            (
                service
                    .library()
                    .projected_upload_attributes(&file, &upload),
                IngestRunSource::Url,
            )
        }
        other => {
            return Ok(terminal(
                "replacement",
                format!("ordinary replacement does not support the {other:?} task kind"),
            ));
        }
    };
    let stored_kind = match super::super::library::storage::detect_file_kind(
        &attributes.filename,
        &attributes.media_type,
    ) {
        Ok(kind) => kind,
        Err(error) => return Ok(terminal("parsing", error.to_string())),
    };
    let sections = if matches!(
        stored_kind,
        crate::services::library::LibraryFileKind::Pdf
            | crate::services::library::LibraryFileKind::Docx
            | crate::services::library::LibraryFileKind::Xlsx
    ) {
        match resolved_sections_with_view(service, task, item, file_id, Some(&attributes)).await? {
            Ok(sections) => sections,
            Err(result) => return Ok(result),
        }
    } else {
        match service
            .library()
            .prepare_task_source_sections_with_view(
                file_id,
                Some(&attributes),
                TaskSourceRef::from_input(item.input_storage_object_id),
                item.lease_token,
            )
            .await
        {
            Ok(sections) => sections,
            Err(error) => return Ok(ingest_failure_result(item, error)),
        }
    };
    let replacement_sections = sections
        .into_iter()
        .enumerate()
        .map(|(index, section)| ReplacementSectionInput {
            section_key: section.section_key,
            section_label: section.section_label,
            sort_order: index as i32,
            section_metadata_json: section.metadata_json,
            title: section.title,
            summary: section.summary,
            body_text: section.body_text,
            source_uri: section.source_uri,
            external_id: section.external_id,
            published_at: section.published_at,
        })
        .collect::<Vec<_>>();
    let staged_object = item.input_storage_object_id;
    let request = ReplacementRunRequest {
        task_id: item.task_id,
        file_id,
        source: run_source,
        staged_source_object: staged_object,
        quota,
        attributes: Some(attributes),
        sections: replacement_sections,
    };
    let fence = TaskWriteFence::from_claimed_item(item)?;
    match service
        .library()
        .stage_task_replacement(service.db(), fence, &request)
        .await
    {
        Ok(TaskIngestStep::Complete(_outcome)) => {
            set_file(service.db(), task.id, item, file_id).await?;
            item.input_storage_object_id = None;
            if let Some(object_id) = staged_object {
                service
                    .library()
                    .release_task_input_staging(object_id, Some(file_id))
                    .await?;
            }
            Ok(ProcessResult::Succeeded(Some(file_id.to_string())))
        }
        Ok(TaskIngestStep::Yielded) => Ok(ProcessResult::Yielded { next: "indexing" }),
        Err(error) => Ok(process_error("replacement", error)),
    }
}

/// Docling sections of an ordinary replacement under the projected file view.
async fn resolved_sections_with_view(
    service: &TaskService,
    task: &crate::db::StoredTask,
    item: &mut ClaimedItem,
    file_id: Uuid,
    view: Option<&crate::services::library::ReplacementFileAttributes>,
) -> anyhow::Result<Result<Vec<IngestSection>, ProcessResult>> {
    let existing = match resolved_section_payload(service, item).await {
        Ok(sections) => sections,
        Err(error) => return Ok(Err(process_error("docling", error))),
    };
    let value = match existing {
        Some(value) => value,
        None => {
            match super::docling_submit::run_docling_submission_segment_with_view(
                service, task.id, item, file_id, view,
            )
            .await?
            {
                ProcessResult::Progressed { .. } => {
                    match resolved_section_payload(service, item).await {
                        Ok(Some(value)) => value,
                        Ok(None) | Err(_) => {
                            return Ok(Err(terminal(
                                "docling",
                                "the replacement source returned no parsed sections",
                            )));
                        }
                    }
                }
                other => return Ok(Err(other)),
            }
        }
    };
    match serde_json::from_value::<Vec<IngestSection>>(value) {
        Ok(sections) => Ok(Ok(sections)),
        Err(error) => Ok(Err(terminal(
            "parsing",
            format!("the replacement source sections are invalid: {error}"),
        ))),
    }
}

pub(super) async fn process_replacement(
    service: &TaskService,
    task: &crate::db::StoredTask,
    group: Option<&crate::domain::GroupRecord>,
    item: &mut ClaimedItem,
) -> anyhow::Result<ProcessResult> {
    let Some(quota) = service.replacement_caps() else {
        return Ok(ProcessResult::Failed {
            stage: "replacement".to_string(),
            message: "source rerun is disabled until the registry capacity policy is configured"
                .to_string(),
            retryable: false,
        });
    };
    let kind = super::responses::parse_kind(&task.kind)?;
    if !matches!(
        kind,
        TaskKind::FileBatch | TaskKind::UrlBatch | TaskKind::TextBatch
    ) {
        return Ok(terminal(
            "replacement",
            format!("source rerun does not support the {kind:?} task kind"),
        ));
    }
    let group = match group {
        Some(group) => group,
        None => return Ok(terminal("replacement", "source rerun requires a group")),
    };
    let Some(file_id) = item.file_id else {
        return Ok(terminal(
            "replacement",
            "source rerun item has no library file",
        ));
    };
    let file = match service.library().file_for_task_source(file_id).await {
        Ok(file) => file,
        Err(error) => return Ok(ingest_failure_result(item, error)),
    };
    if file.group_id != group.id {
        return Ok(terminal(
            "replacement",
            "source rerun file belongs to another group",
        ));
    }
    if crate::services::library::is_managed_source_file(&file.filename, file.external_id.as_deref())
    {
        return Ok(terminal(
            "replacement",
            "managed source-folder files cannot be replaced by a source rerun",
        ));
    }
    let stored_kind =
        match super::super::library::storage::detect_file_kind(&file.filename, &file.media_type) {
            Ok(kind) => kind,
            Err(error) => return Ok(terminal("parsing", error.to_string())),
        };

    if kind == TaskKind::UrlBatch && item.input_storage_object_id.is_none() {
        if let Some(result) =
            materialize_fresh_url(service, group, item, &file, stored_kind).await?
        {
            return Ok(result);
        }
    }

    let sections = if matches!(
        stored_kind,
        crate::services::library::LibraryFileKind::Pdf
            | crate::services::library::LibraryFileKind::Docx
            | crate::services::library::LibraryFileKind::Xlsx
    ) {
        match resolved_sections(service, task, item, file_id).await? {
            Ok(sections) => sections,
            Err(result) => return Ok(result),
        }
    } else {
        let source = TaskSourceRef::from_input(item.input_storage_object_id);
        match service
            .library()
            .prepare_task_source_sections(file_id, source, item.lease_token)
            .await
        {
            Ok(sections) => sections,
            Err(error) => return Ok(ingest_failure_result(item, error)),
        }
    };

    let replacement_sections = sections
        .into_iter()
        .enumerate()
        .map(|(index, section)| ReplacementSectionInput {
            section_key: section.section_key,
            section_label: section.section_label,
            sort_order: index as i32,
            section_metadata_json: section.metadata_json,
            title: section.title,
            summary: section.summary,
            body_text: section.body_text,
            source_uri: section.source_uri,
            external_id: section.external_id,
            published_at: section.published_at,
        })
        .collect::<Vec<_>>();

    let staged_object = item.input_storage_object_id;
    let request = ReplacementRunRequest {
        task_id: item.task_id,
        file_id,
        source: if kind == TaskKind::UrlBatch {
            IngestRunSource::Url
        } else {
            IngestRunSource::Stored
        },
        staged_source_object: staged_object,
        quota,
        attributes: None,
        sections: replacement_sections,
    };
    let fence = TaskWriteFence::from_claimed_item(item)?;
    match service
        .library()
        .stage_task_replacement(service.db(), fence, &request)
        .await
    {
        Ok(TaskIngestStep::Complete(_outcome)) => {
            // Only after the switch committed is the staged reference cleared
            // and the staged object bound to the live file.
            set_file(service.db(), task.id, item, file_id).await?;
            item.input_storage_object_id = None;
            if let Some(object_id) = staged_object {
                service
                    .library()
                    .release_task_input_staging(object_id, Some(file_id))
                    .await?;
            }
            Ok(ProcessResult::Succeeded(Some(file_id.to_string())))
        }
        Ok(TaskIngestStep::Yielded) => Ok(ProcessResult::Yielded { next: "indexing" }),
        Err(error) => Ok(process_error("replacement", error)),
    }
}

/// Resolve the docling sections of this new item only.
///
/// The completion of the new item's own job persists its sections into its own
/// payload. The old embedding-next stage is ignored: only the fresh sections
/// are returned, and a parent's saved parse is never consulted.
async fn resolved_sections(
    service: &TaskService,
    task: &crate::db::StoredTask,
    item: &mut ClaimedItem,
    file_id: Uuid,
) -> anyhow::Result<Result<Vec<IngestSection>, ProcessResult>> {
    let existing = match resolved_section_payload(service, item).await {
        Ok(sections) => sections,
        Err(error) => return Ok(Err(process_error("docling", error))),
    };
    let value = match existing {
        Some(value) => value,
        None => {
            match super::docling_submit::run_docling_submission_segment(
                service, task.id, item, file_id,
            )
            .await?
            {
                ProcessResult::Progressed { .. } => {
                    match resolved_section_payload(service, item).await {
                        Ok(Some(value)) => value,
                        Ok(None) | Err(_) => {
                            return Ok(Err(terminal(
                                "docling",
                                "the new source returned no parsed sections",
                            )));
                        }
                    }
                }
                other => return Ok(Err(other)),
            }
        }
    };
    match serde_json::from_value::<Vec<IngestSection>>(value) {
        Ok(sections) => Ok(Ok(sections)),
        Err(error) => Ok(Err(terminal(
            "parsing",
            format!("the new source sections are invalid: {error}"),
        ))),
    }
}

/// Fetch fresh bytes for a URL source and attach them as this item's input.
///
/// The URL always comes from the live library file, never from an arbitrary new
/// URL in the forked payload. A download whose detected kind changed is refused
/// before any parse, and the staged object is released on every refusal path.
async fn materialize_fresh_url(
    service: &TaskService,
    group: &crate::domain::GroupRecord,
    item: &mut ClaimedItem,
    file: &crate::domain::LibraryFileRecord,
    stored_kind: crate::services::library::LibraryFileKind,
) -> anyhow::Result<Option<ProcessResult>> {
    let Some(url) = file.source_uri.clone() else {
        return Ok(Some(terminal(
            "download",
            "the library file has no source URL to fetch again",
        )));
    };
    let request = ImportLibraryFileFromUrlRequest {
        url,
        title: None,
        folder_id: file.folder_id,
        filename: Some(file.filename.clone()),
        media_type: Some(file.media_type.clone()),
        options: url_options(&item.payload),
    };
    let materialized = match service
        .library()
        .materialize_url_source_for_task(group.id, &request, item.lease_token)
        .await
    {
        Ok(materialized) => materialized,
        Err(error) => return Ok(Some(process_error("download", error))),
    };
    let fresh_kind = crate::services::library::storage::detect_file_kind(
        &materialized.filename,
        &materialized.media_type,
    );
    if fresh_kind.as_ref().ok() != Some(&stored_kind) {
        let _ = service
            .library()
            .release_task_input_staging(materialized.object_id, None)
            .await;
        return Ok(Some(terminal(
            "download",
            format!(
                "the fresh URL source changed file kind ({} -> {})",
                stored_kind_name(stored_kind),
                materialized.filename
            ),
        )));
    }
    let Some(execution) = item.execution_context() else {
        let _ = service
            .library()
            .release_task_input_staging(materialized.object_id, None)
            .await;
        return Ok(Some(terminal(
            "download",
            "task item has no captured execution",
        )));
    };
    let payload = item.payload.clone();
    if !service
        .db()
        .set_task_item_input_storage_object(item.id, &execution, materialized.object_id, &payload)
        .await?
    {
        let _ = service
            .library()
            .release_task_input_staging(materialized.object_id, None)
            .await;
        return Ok(Some(terminal(
            "download",
            "task item lease was lost while saving staged input",
        )));
    }
    item.input_storage_object_id = Some(materialized.object_id);
    Ok(None)
}

fn url_options(payload: &Value) -> IngestOptions {
    payload
        .get("options")
        .and_then(|options| serde_json::from_value::<IngestOptions>(options.clone()).ok())
        .unwrap_or_else(IngestOptions::retain)
}

fn stored_kind_name(kind: crate::services::library::LibraryFileKind) -> &'static str {
    match kind {
        crate::services::library::LibraryFileKind::Pdf => "pdf",
        crate::services::library::LibraryFileKind::Docx => "docx",
        crate::services::library::LibraryFileKind::Xlsx => "xlsx",
        crate::services::library::LibraryFileKind::PlainText => "plain_text",
    }
}

fn terminal(stage: &str, message: impl ToString) -> ProcessResult {
    ProcessResult::Failed {
        stage: stage.to_string(),
        message: message.to_string(),
        retryable: false,
    }
}

/// Map a source preparation failure onto the worker outcome.
///
/// A retryable dependency failure parks the item with the existing waiting
/// shape; only a genuinely terminal failure marks it failed.
fn ingest_failure_result(
    item: &ClaimedItem,
    error: crate::services::library::UnifiedIngestError,
) -> ProcessResult {
    if error.retryable {
        waiting_for_error(item, error)
    } else {
        ProcessResult::Failed {
            stage: error.stage,
            message: error.message,
            retryable: false,
        }
    }
}
