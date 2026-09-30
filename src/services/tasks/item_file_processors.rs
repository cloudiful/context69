use crate::domain_errors::DomainError;

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::{LibraryIngestStatus, UpsertLibraryTextRequest};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{
    ProcessResult, persisted_section_payload, process_error, save_sections, set_file,
    waiting_for_error,
};
use crate::services::library::storage;
use crate::services::library::{UnifiedIngestError, UploadedLibraryFile};

pub(super) async fn process_text(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context(DomainError::invalid_argument("text tasks require group_id"))?;
    if stage == "storage" {
        // Recovery: `save_sections` stores the parsed sections back into the
        // item payload as `section_payload`, which `UpsertLibraryTextRequest`
        // rejects as an unknown field. A resumed item whose storage already
        // persisted sections therefore cannot be re-parsed as the initial
        // request; skip the duplicate upsert and index what is already stored.
        if let Some(next) = text_storage_resume_stage(&item.payload) {
            return Ok(ProcessResult::Progressed { next });
        }
        let request: UpsertLibraryTextRequest = match serde_json::from_value(item.payload.clone()) {
            Ok(request) => request,
            Err(error) => return Ok(process_error(stage, error.into())),
        };
        let (file, section_payload) = match service
            .library()
            .upsert_text_file_for_task(group, &request, item.lease_token)
            .await
        {
            Ok(result) => result,
            Err(error) => return Ok(process_error(stage, error)),
        };
        set_file(service.db(), task.id, item, file.file_id).await?;
        // The storage stage saved the sections; the shared snapshot carries
        // them so the indexing stage does not re-derive them.
        save_sections(service.db(), item, section_payload).await?;
        return Ok(ProcessResult::Progressed { next: "indexing" });
    }
    process_file_stage(service, group.id, item, stage).await
}

/// Where a TextBatch item resumes when its storage stage already persisted the
/// sections. `save_sections` writes them into the item payload as
/// `section_payload`, which is not part of the canonical
/// `UpsertLibraryTextRequest` (`deny_unknown_fields`), so re-parsing the stored
/// request would fail with `unknown field section_payload` and loop retries.
/// Only a non-null persisted section payload counts as completed storage.
fn text_storage_resume_stage(payload: &Value) -> Option<&'static str> {
    persisted_section_payload(payload).map(|_| "indexing")
}

pub(super) async fn process_file(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context(DomainError::invalid_argument("file tasks require group_id"))?;
    if stage == "storage" {
        let file = if let Some(file_id) = item.file_id {
            match service
                .library()
                .file_summary_for_task(group.id, file_id)
                .await
            {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, error)),
            }
        } else {
            let request: StoredFileBatchItem = match serde_json::from_value(item.payload.clone()) {
                Ok(request) => request,
                Err(error) => return Ok(process_error(stage, error.into())),
            };
            let bytes = if let Some(object_id) = item.input_storage_object_id {
                match service
                    .library()
                    .read_task_input_for_task(group.id, object_id, item.lease_token)
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(error) => return Ok(process_error(stage, error)),
                }
            } else {
                match STANDARD.decode(request.content_base64.as_deref().unwrap_or_default().trim())
                {
                    Ok(bytes) => bytes.into(),
                    Err(error) => return Ok(process_error(stage, anyhow!(error))),
                }
            };
            let ingest = &request.options;
            match service
                .library()
                .prepare_file_for_task(
                    group.id,
                    UploadedLibraryFile {
                        folder_id: request.folder_id,
                        filename: request.filename,
                        media_type: request.media_type,
                        bytes,
                        declared_sha256: request.declared_sha256,
                        options: ingest.clone(),
                        staged_storage_object_id: item.input_storage_object_id,
                    },
                    item.lease_token,
                )
                .await
            {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, error)),
            }
        };
        set_file(service.db(), task.id, item, file.file_id).await?;
        if let Some(object_id) = item.input_storage_object_id {
            service
                .library()
                .release_task_input_staging(object_id, Some(file.file_id))
                .await?;
        }
        let next = if file.ingest_status == LibraryIngestStatus::Succeeded {
            // The file is already ingested (URL task reusing an existing
            // file, or a resumed item): only translation/extraction are left.
            "translation"
        } else {
            service
                .library()
                .file_ingest_stage(&file.filename, &file.media_type)?
        };
        return Ok(ProcessResult::Progressed { next });
    }
    process_file_stage(service, group.id, item, stage).await
}

/// v0.18 stored worker payload: canonical `options` is required. Legacy
/// flattened payloads (no `options`, or `metadata`/`translation`/
/// `extraction`/`delete_source_after_processing` keys) are rejected at
/// deserialization so in-flight v0.15 rows fail closed instead of silently
/// changing ingest semantics. `rerun`/`retry` copy the stored JSON verbatim
/// and never convert; the worker is the rejection boundary.
///
/// Rerun/unfinished audit: `rerun_task` (`rerun_items.sql`) and the
/// unfinished-slot lock (`unfinished_item_file_ids.sql`) select only
/// `payload`/`file_id` and never deserialize into this struct, so they copy
/// legacy bytes without interpreting them. Rejection happens here on the
/// next worker claim.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFileBatchItem {
    filename: String,
    #[serde(default)]
    media_type: String,
    #[serde(default)]
    content_base64: Option<String>,
    #[serde(default)]
    declared_sha256: Option<String>,
    #[serde(default)]
    folder_id: Option<Uuid>,
    options: context69_contracts::IngestOptions,
}

pub(super) async fn process_file_stage(
    service: &TaskService,
    group_id: i64,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let file_id = item.file_id.context(DomainError::invalid_argument(
        "file task stage requires file_id",
    ))?;
    match stage {
        "docling" => {
            if persisted_section_payload(&item.payload).is_some() {
                return Ok(ProcessResult::Progressed { next: "embedding" });
            }
            let file = service
                .library()
                .file_summary_for_task(group_id, file_id)
                .await?;
            if file.ingest_status == LibraryIngestStatus::Succeeded {
                return Ok(ProcessResult::Progressed {
                    next: "translation",
                });
            }
            // Blocking conversion (issue 650 P3): submit, status polling, and
            // result fetching run inline inside the admitted parent slot. The
            // worker keeps its item lease for the whole remote wait; a crash
            // resumes from the persisted remote id without resubmitting.
            return super::docling_submit::submit_and_await_blocking(
                service,
                item.task_id,
                item,
                file_id,
            )
            .await;
        }
        "embedding" => {
            if persisted_section_payload(&item.payload).is_none() {
                // Recovery: without a section payload the file still needs
                // conversion. Docling files advance back to the blocking
                // `docling` stage instead of converting while holding worker
                // capacity ambiguously; plain text has no remote conversion
                // and keeps preparing inline.
                let file = service
                    .library()
                    .file_summary_for_task(group_id, file_id)
                    .await?;
                // An undetectable kind falls through to the inline prepare,
                // which reports the parsing failure exactly as before.
                let recovery = storage::detect_file_kind(&file.filename, &file.media_type)
                    .ok()
                    .and_then(|kind| kind.recovery_stage_for_missing_sections());
                if let Some(stage) = recovery {
                    return Ok(ProcessResult::Progressed { next: stage });
                }
                let sections = match service
                    .library()
                    .prepare_file_sections_for_task(file_id, item.lease_token, item.task_id, None)
                    .await
                {
                    Ok(sections) => sections,
                    Err(error) => return ingest_error_result(service, item, file_id, error).await,
                };
                save_sections(service.db(), item, sections).await?;
            }
            Ok(ProcessResult::Progressed { next: "indexing" })
        }
        "indexing" => {
            let file = service
                .library()
                .file_summary_for_task(group_id, file_id)
                .await?;
            if file.ingest_status == LibraryIngestStatus::Succeeded {
                return Ok(ProcessResult::Progressed {
                    next: "translation",
                });
            }
            let sections = match persisted_section_payload(&item.payload) {
                Some(sections) => sections,
                None => {
                    // Same recovery as the embedding stage: a Docling file
                    // without sections goes back to the blocking `docling`
                    // stage instead of converting inline. An undetectable
                    // kind falls through to the inline prepare, which reports
                    // the parsing failure exactly as before.
                    let recovery = storage::detect_file_kind(&file.filename, &file.media_type)
                        .ok()
                        .and_then(|kind| kind.recovery_stage_for_missing_sections());
                    if let Some(stage) = recovery {
                        return Ok(ProcessResult::Progressed { next: stage });
                    }
                    match service
                        .library()
                        .prepare_file_sections_for_task(
                            file_id,
                            item.lease_token,
                            item.task_id,
                            None,
                        )
                        .await
                    {
                        Ok(sections) => {
                            save_sections(service.db(), item, sections.clone()).await?;
                            sections
                        }
                        Err(error) => {
                            return ingest_error_result(service, item, file_id, error).await;
                        }
                    }
                }
            };
            if let Err(error) = service
                .library()
                .persist_file_sections_for_task_with_checkpoint(
                    file_id,
                    &sections,
                    item.id,
                    item.lease_token,
                    &item.payload,
                )
                .await
            {
                return ingest_error_result(service, item, file_id, error).await;
            }
            Ok(ProcessResult::Progressed {
                next: "translation",
            })
        }
        "translation" => {
            if let Err(error) = service.library().convert_file_translations(file_id).await {
                return Ok(process_error(stage, error));
            }
            Ok(ProcessResult::Progressed { next: "extraction" })
        }
        "extraction" => {
            if let Err(error) = service.library().convert_file_extractions(file_id).await {
                return Ok(process_error(stage, error));
            }
            Ok(ProcessResult::Progressed { next: "finalize" })
        }
        "finalize" => Ok(ProcessResult::Succeeded(Some(file_id.to_string()))),
        other => Ok(process_error(
            other,
            DomainError::invalid_argument(format!("unsupported file task stage {other}")).into(),
        )),
    }
}

async fn ingest_error_result(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
    file_id: Uuid,
    error: UnifiedIngestError,
) -> Result<ProcessResult> {
    let error = service
        .library()
        .handle_task_ingest_failure_with_payload(
            file_id,
            item.lease_token,
            error,
            Some(&item.payload),
        )
        .await;
    if error.retryable {
        Ok(waiting_for_error(item, error))
    } else {
        Ok(ProcessResult::Failed {
            stage: error.stage,
            message: error.message,
            retryable: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{StoredFileBatchItem, text_storage_resume_stage};
    use context69_contracts::UpsertLibraryTextRequest;

    /// Regression for issue 662: a TextBatch item that already persisted its
    /// sections (`save_sections` wrote `section_payload`) must resume at
    /// `indexing` instead of being re-parsed as the initial upsert request,
    /// which rejects the extra key and looped storage retries.
    #[test]
    fn text_storage_resumes_at_indexing_once_sections_are_persisted() {
        let persisted = serde_json::json!({
            "external_id": "text-1",
            "title": "Persisted text",
            "content": "body",
            "section_payload": [{ "section_key": "document" }],
        });
        // The stored request is no longer parseable as the canonical upsert:
        // this is the `unknown field section_payload` failure being recovered.
        assert!(
            serde_json::from_value::<UpsertLibraryTextRequest>(persisted.clone()).is_err(),
            "the persisted payload must not parse as the initial upsert request"
        );
        assert_eq!(text_storage_resume_stage(&persisted), Some("indexing"));

        // A null section payload is not a completed storage stage.
        let not_stored = serde_json::json!({
            "external_id": "text-2",
            "title": "Fresh text",
            "content": "body",
            "section_payload": null,
        });
        assert_eq!(text_storage_resume_stage(&not_stored), None);
    }

    /// The recovery guard must not weaken the initial storage contract: a
    /// payload without persisted sections still parses as the canonical
    /// request, and genuine unknown fields are still rejected.
    #[test]
    fn text_storage_keeps_the_strict_initial_parse_contract() {
        let fresh = serde_json::json!({
            "external_id": "text-3",
            "title": "Fresh text",
            "content": "body",
        });
        assert_eq!(text_storage_resume_stage(&fresh), None);
        let request: UpsertLibraryTextRequest = serde_json::from_value(fresh)
            .expect("a fresh payload still parses as the initial upsert request");
        assert_eq!(request.external_id, "text-3");

        assert!(
            serde_json::from_value::<UpsertLibraryTextRequest>(serde_json::json!({
                "external_id": "text-4",
                "title": "Invalid text",
                "content": "body",
                "totally_unknown": true,
            }))
            .is_err(),
            "unknown fields must still be rejected"
        );
    }

    #[test]
    fn stored_file_payload_requires_canonical_options_and_rejects_legacy() {
        // Canonical stored payload parses and carries the ingest policy.
        let canonical: StoredFileBatchItem = serde_json::from_value(serde_json::json!({
            "filename": "a.pdf",
            "media_type": "application/pdf",
            "content_base64": "aGk=",
            "options": { "metadata": { "external_id": "doc-2" }, "source_policy": "retain" }
        }))
        .expect("canonical stored payload");
        assert!(!canonical.options.is_release());
        assert_eq!(
            canonical.options.metadata.external_id.as_deref(),
            Some("doc-2")
        );

        // Staged payloads without inline bytes (content lives in the staged
        // storage object) also parse: content_base64 stays optional.
        let staged: StoredFileBatchItem = serde_json::from_value(serde_json::json!({
            "filename": "a.pdf",
            "media_type": "application/pdf",
            "options": { "metadata": {}, "source_policy": "release_after_processing" }
        }))
        .expect("staged stored payload");
        assert!(staged.options.is_release());
        assert!(staged.content_base64.is_none());

        // v0.15 in-flight payloads without `options` are rejected (fail
        // closed): rerun copies bytes verbatim, the worker rejects here.
        assert!(
            serde_json::from_value::<StoredFileBatchItem>(serde_json::json!({
                "filename": "a.pdf",
                "media_type": "application/pdf",
                "content_base64": "aGk=",
                "metadata": { "external_id": "doc-1", "metadata_json": { "k": "v" } },
                "delete_source_after_processing": true
            }))
            .is_err(),
            "legacy stored payload without options must be rejected"
        );
        // Flattened duplicates alongside canonical are also rejected.
        assert!(
            serde_json::from_value::<StoredFileBatchItem>(serde_json::json!({
                "filename": "a.pdf",
                "media_type": "application/pdf",
                "content_base64": "aGk=",
                "options": { "metadata": { "external_id": "doc-2" }, "source_policy": "retain" },
                "metadata": { "external_id": "doc-1", "metadata_json": {} },
                "delete_source_after_processing": true
            }))
            .is_err(),
            "flattened duplicates must be rejected"
        );
        // Non-object canonical metadata is rejected at the type boundary.
        assert!(
            serde_json::from_value::<StoredFileBatchItem>(serde_json::json!({
                "filename": "a.pdf",
                "media_type": "application/pdf",
                "content_base64": "aGk=",
                "options": { "metadata": { "metadata_json": [1, 2] }, "source_policy": "retain" }
            }))
            .is_err(),
            "non-object metadata_json must be rejected"
        );
    }
}
