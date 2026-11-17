//! The image ingestion item pipeline.
//!
//! One item stores one image file, optionally links it to one parent document
//! occurrence in the task group, then embeds the stored image bytes and upserts
//! one image point. It never creates text documents or text chunks: an image is
//! searchable only through its own image vector.

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::tasks::ImageBatchItem;

use super::TaskService;
use super::item_processors::{ProcessResult, process_error};
use super::pipeline_progress::record_stage;
use super::pipeline_progress_input::{
    deserialize_worker_input, normalize_image_source, payload_with_image_expected_sha256,
    payload_with_resolved_image_source, persisted_image_expected_sha256,
    persisted_image_resolved_filename, persisted_image_resolved_media_type,
};
use crate::config::QdrantConfig;
use crate::image_embedding_vectors::ImageVectorPoint;
use crate::services::library::{ImageLinkRequest, PipelineProgressStage, UploadedLibraryFile};

/// Whether a mutable file row is still the submission's bound revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageRevisionState {
    /// The row carries the exact content revision staged for this item.
    Bound,
    /// Another writer replaced the row; this submission must never claim it.
    Superseded,
    /// The item has no durable staged revision, so no replay can be trusted.
    Missing,
}

fn image_revision_state(expected: Option<&str>, current: &str) -> ImageRevisionState {
    match expected {
        Some(expected) if expected == current => ImageRevisionState::Bound,
        Some(_) => ImageRevisionState::Superseded,
        None => ImageRevisionState::Missing,
    }
}

/// The explicit failure for a replay whose bound revision no longer matches.
fn image_revision_failure(stage: &str, state: ImageRevisionState) -> ProcessResult {
    let message = match state {
        ImageRevisionState::Superseded => {
            "the image file was replaced by another submission revision; this task cannot claim \
             the newer content"
        }
        ImageRevisionState::Missing => {
            "the image item has no durable staged content revision; refusing to infer the latest \
             file content"
        }
        ImageRevisionState::Bound => return ProcessResult::Progressed { next: "embedding" },
    };
    ProcessResult::Failed {
        stage: stage.to_string(),
        message: message.to_string(),
        retryable: false,
    }
}

pub(super) async fn process_image(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context(crate::domain_errors::DomainError::invalid_argument(
        "image tasks require group_id",
    ))?;
    match stage {
        "storage" => {
            let mut request: ImageBatchItem = match deserialize_worker_input(&item.payload) {
                Ok(request) => request,
                Err(error) => return Ok(process_error(stage, error.into())),
            };
            // Defensive replay boundary: an already-persisted payload may still
            // carry a whitespace-only URL/inline field. Normalize the source the
            // same way the submit dispatcher does, without re-running the public
            // XOR validation on worker-owned (already stripped) staged input.
            normalize_image_source(&mut request);
            if let Err(error) =
                crate::services::library::ensure_image_options_supported(&request.options)
            {
                return Ok(process_error(stage, error));
            }
            if let Some(file_id) = item.file_id {
                let file = match service.library().file_for_task_source(file_id).await {
                    Ok(file) => file,
                    Err(error) => return Ok(process_error(stage, anyhow::Error::from(error))),
                };
                let state = image_revision_state(
                    persisted_image_expected_sha256(&item.payload),
                    &file.sha256,
                );
                if state != ImageRevisionState::Bound {
                    return Ok(image_revision_failure(stage, state));
                }
                // A replayed item always proves its parent occurrence link
                // again: the stored file id is not a link acknowledgement.
                if let Err(error) =
                    ensure_image_parent_link(service, group.id, file_id, &request).await
                {
                    return Ok(process_error(stage, error));
                }
                return Ok(ProcessResult::Progressed { next: "embedding" });
            }
            let (upload, expected_sha256) =
                match image_upload(service, item, group.id, &request).await {
                    Ok(upload) => upload,
                    Err(error) => return Ok(process_error(stage, error)),
                };
            // The actual staged revision is durable before any file id: a
            // reclaimed item never falls back to whatever the mutable file row
            // happens to contain. A missing or contradicting staged source was
            // already refused by `image_upload`.
            let payload = payload_with_image_expected_sha256(&item.payload, &expected_sha256);
            let fence = match crate::library_store::TaskWriteFence::from_claimed_item(item) {
                Ok(fence) => fence,
                Err(error) => return Ok(process_error(stage, error)),
            };
            match crate::library_store::write_task_item_payload(service.db(), fence, &payload).await
            {
                Ok(true) => item.payload = payload,
                Ok(false) => {
                    return Ok(process_error(
                        stage,
                        crate::domain_errors::DomainError::conflict(
                            "the image item lease was lost while binding its staged revision",
                        )
                        .into(),
                    ));
                }
                Err(error) => return Ok(process_error(stage, error)),
            }
            let staged_object_id = upload.staged_storage_object_id;
            let fence = match crate::library_store::TaskWriteFence::from_claimed_item(item) {
                Ok(fence) => fence,
                Err(error) => return Ok(process_error(stage, error)),
            };
            let file = match service
                .library()
                .store_image_file_for_task_item(
                    group.id,
                    upload,
                    item.lease_token,
                    fence,
                    task.id,
                    item.id,
                )
                .await
            {
                Ok(file) => file,
                Err(error) => {
                    if let Some(object_id) = staged_object_id {
                        let _ = service
                            .library()
                            .release_task_input_staging(object_id, None)
                            .await;
                    }
                    return Ok(process_error(stage, error));
                }
            };
            item.file_id = Some(file.file_id);
            item.input_storage_object_id = None;
            if let Err(error) =
                ensure_image_parent_link(service, group.id, file.file_id, &request).await
            {
                return Ok(process_error(stage, error));
            }
            Ok(ProcessResult::Progressed { next: "embedding" })
        }
        "embedding" => {
            record_stage(service.db(), item, PipelineProgressStage::Indexing).await?;
            let Some(snapshot) = service.settings().image_embedding_snapshot() else {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "image embedding is disabled; enable it in runtime settings before \
                              ingesting images"
                        .to_string(),
                    retryable: false,
                });
            };
            let Some(expected_sha256) = persisted_image_expected_sha256(&item.payload) else {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "image item has no durable staged content revision".to_string(),
                    retryable: false,
                });
            };
            let Some(file_id) = item.file_id else {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "image item has no stored library file".to_string(),
                    retryable: false,
                });
            };
            let file = match service.library().file_for_task_source(file_id).await {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, anyhow::Error::from(error))),
            };
            if file.group_id != group.id {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "the image file belongs to another group".to_string(),
                    retryable: false,
                });
            }
            let revision = image_revision_state(Some(expected_sha256), &file.sha256);
            if revision != ImageRevisionState::Bound {
                return Ok(image_revision_failure(stage, revision));
            }
            let declared = match crate::services::library::require_image_media(
                &file.filename,
                &file.media_type,
            ) {
                Ok(kind) => kind,
                Err(error) => {
                    return Ok(ProcessResult::Failed {
                        stage: "embedding".to_string(),
                        message: format!("the image file row is not a supported image: {error}"),
                        retryable: false,
                    });
                }
            };
            let bytes = match service
                .library()
                .read_library_file_bytes_for_lease(&file, item.lease_token)
                .await
            {
                Ok(bytes) => bytes,
                Err(error) => return Ok(process_error(stage, error)),
            };
            if crate::services::library::image_kind_from_bytes(&bytes) != Some(declared) {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "the image file bytes do not match the stored image kind".to_string(),
                    retryable: false,
                });
            }
            if crate::services::library::hash_bytes(&bytes) != expected_sha256 {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "the image bytes are not the content revision staged for this item"
                        .to_string(),
                    retryable: false,
                });
            }
            let mut vectors = match snapshot.provider.embed_images(&[bytes.to_vec()]).await {
                Ok(vectors) => vectors,
                Err(error) => return Ok(process_error(stage, error)),
            };
            let vector = match vectors.pop() {
                Some(vector) => vector,
                None => {
                    return Ok(ProcessResult::Failed {
                        stage: "embedding".to_string(),
                        message: "the image provider returned no vector".to_string(),
                        retryable: false,
                    });
                }
            };
            let Some(current_snapshot) = service.settings().image_embedding_snapshot() else {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "image embedding was disabled while the item was running".to_string(),
                    retryable: true,
                });
            };
            if current_snapshot.fingerprint != snapshot.fingerprint {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "the image embedding runtime changed; retry the item against the \
                              current runtime"
                        .to_string(),
                    retryable: true,
                });
            }
            // The complete destination is captured with the provider snapshot.
            // It is never rebuilt from mutable runtime settings between
            // collection preparation and the final publication.
            let Some(target) = snapshot.destination.clone() else {
                return Ok(ProcessResult::Failed {
                    stage: "embedding".to_string(),
                    message: "the image vector destination is not configured".to_string(),
                    retryable: false,
                });
            };
            let config = QdrantConfig {
                url: target.endpoint.clone(),
                collection_name: target.collection.clone(),
                recreate_on_dimension_mismatch: false,
            };
            let index = match crate::qdrant_index::QdrantIndex::ensure_image_collection(
                &config,
                &target.collection,
                snapshot.config.dimensions,
                &target.runtime_fingerprint,
            )
            .await
            {
                Ok(index) => index,
                Err(error) => return Ok(process_error(stage, error)),
            };
            let publish_attempt_id = uuid::Uuid::new_v4();
            let point = ImageVectorPoint {
                point_id: crate::image_embedding_vectors::image_point_id(
                    file.id,
                    expected_sha256,
                    &target.runtime_fingerprint,
                    publish_attempt_id,
                ),
                publish_attempt_id,
                file_id: file.id,
                group_id: file.group_id,
                sha256: expected_sha256.to_string(),
                runtime_fingerprint: target.runtime_fingerprint.clone(),
                media_type: file.media_type,
                external_id: file.external_id,
                vector,
            };
            if let Err(error) = service
                .library()
                .publish_image_vector(crate::services::library::ImageVectorPublish {
                    group_id: group.id,
                    item,
                    attempt_id: publish_attempt_id,
                    expected_sha256,
                    point: &point,
                    target: &target,
                    index: &index,
                })
                .await
            {
                return Ok(process_error(stage, error));
            }
            Ok(ProcessResult::Progressed { next: "finalize" })
        }
        "finalize" => {
            record_stage(service.db(), item, PipelineProgressStage::Finalizing).await?;
            let file_id = item
                .file_id
                .context(anyhow!("image item has no stored file"))?;
            // The file status was acknowledged inside the guarded publish
            // transaction for that attempt's captured SHA and target, so the
            // final stage only completes the item and reports the same identity.
            Ok(ProcessResult::Succeeded(Some(file_id.to_string())))
        }
        other => Ok(process_error(
            other,
            crate::domain_errors::DomainError::invalid_argument(format!(
                "unsupported image task stage {other}"
            ))
            .into(),
        )),
    }
}

/// The nonblank URL of an image item, treating whitespace-only as absent.
///
/// The worker uses this single accessor instead of raw `Option` checks so a
/// persisted whitespace-only URL can never select the URL replay branch.
fn nonblank_url(request: &ImageBatchItem) -> Option<&str> {
    request
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
}

/// The file identity of a staged image replay from the normalized request.
///
/// A nonblank URL selects the URL branch: persisted resolved metadata wins,
/// then full request fields, else the missing-metadata conflict. A staged inline
/// item uses its own filename/media type directly, so a whitespace-only URL in
/// an old persisted payload cannot divert it into the URL branch.
fn staged_replay_identity(
    request: &ImageBatchItem,
    resolved_filename: Option<&str>,
    resolved_media_type: Option<&str>,
) -> Result<(String, String)> {
    if nonblank_url(request).is_none() {
        return Ok((request.filename.clone(), request.media_type.clone()));
    }
    match (resolved_filename, resolved_media_type) {
        (Some(filename), Some(media_type)) => Ok((filename.to_string(), media_type.to_string())),
        _ if !request.filename.trim().is_empty() && !request.media_type.trim().is_empty() => {
            Ok((request.filename.clone(), request.media_type.clone()))
        }
        _ => Err(crate::domain_errors::DomainError::conflict(
            "the bound URL image source has no resolved filename and media type",
        )
        .into()),
    }
}

/// Ensure the parent occurrence link of one image item, including a replay.
///
/// The stored file id is never treated as a link acknowledgement: a retry after
/// a crash between storage and linking re-runs this idempotent write. A missing
/// or deleted parent fails explicitly; no placeholder document is created.
async fn ensure_image_parent_link(
    service: &TaskService,
    group_id: i64,
    file_id: uuid::Uuid,
    request: &ImageBatchItem,
) -> Result<()> {
    let Some(parent) = request.parent.as_ref() else {
        return Ok(());
    };
    let role = request.role.unwrap_or_default();
    let source_uri = request
        .source_uri
        .as_deref()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .or_else(|| nonblank_url(request));
    let link = ImageLinkRequest {
        parent,
        role,
        ordinal: request.ordinal,
        caption: request.caption.as_deref(),
        context: request.context.as_deref(),
        source_uri,
    };
    service
        .library()
        .link_image_to_document(group_id, file_id, &link)
        .await?;
    Ok(())
}

async fn image_upload(
    service: &TaskService,
    item: &mut crate::db::ClaimedItem,
    group_id: i64,
    request: &ImageBatchItem,
) -> Result<(UploadedLibraryFile, String)> {
    let persisted = persisted_image_expected_sha256(&item.payload).map(str::to_string);
    if let Some(object_id) = item.input_storage_object_id {
        let Some(expected_sha256) = persisted else {
            return Err(crate::domain_errors::DomainError::conflict(
                "the staged image source has no durable submission revision",
            )
            .into());
        };
        let actual = service
            .library()
            .staged_task_input_sha256(group_id, object_id)
            .await?;
        if actual != expected_sha256 {
            return Err(crate::domain_errors::DomainError::conflict(
                "the staged image source no longer matches its recorded submission revision",
            )
            .into());
        }
        let bytes = service
            .library()
            .read_task_input_for_task(group_id, object_id, item.lease_token)
            .await?;
        if crate::services::library::hash_bytes(&bytes) != expected_sha256 {
            return Err(crate::domain_errors::DomainError::conflict(
                "the staged image bytes do not match their recorded submission revision",
            )
            .into());
        }
        let (filename, media_type) = staged_replay_identity(
            request,
            persisted_image_resolved_filename(&item.payload),
            persisted_image_resolved_media_type(&item.payload),
        )?;
        return Ok((
            UploadedLibraryFile {
                folder_id: request.folder_id,
                filename,
                media_type,
                bytes,
                declared_sha256: request.declared_sha256.clone(),
                options: request.options.clone(),
                staged_storage_object_id: Some(object_id),
            },
            expected_sha256,
        ));
    }
    if let Some(url) = nonblank_url(request) {
        if persisted.is_some() {
            return Err(crate::domain_errors::DomainError::conflict(
                "the image item is bound to a staged revision whose source object is missing",
            )
            .into());
        }
        let materialized = service
            .library()
            .materialize_url_source_for_task(
                group_id,
                &crate::contracts::ImportLibraryFileFromUrlRequest {
                    url: url.to_string(),
                    title: None,
                    folder_id: request.folder_id,
                    filename: Some(request.filename.clone())
                        .filter(|value| !value.trim().is_empty()),
                    media_type: Some(request.media_type.clone())
                        .filter(|value| !value.trim().is_empty()),
                    options: request.options.clone(),
                },
                item.lease_token,
            )
            .await?;
        let filename = if request.filename.trim().is_empty() {
            materialized.filename.clone()
        } else {
            request.filename.clone()
        };
        let media_type = if request.media_type.trim().is_empty() {
            materialized.media_type.clone()
        } else {
            request.media_type.clone()
        };
        // This one guarded statement persists the staged object, its actual
        // downloaded SHA, and the resolved source metadata together, so every
        // retry resumes the same immutable source instead of refetching a
        // mutable URL or falling back to empty request fields.
        let payload = payload_with_resolved_image_source(
            &item.payload,
            &materialized.sha256,
            &filename,
            &media_type,
        );
        let Some(execution) = item.execution_context() else {
            let _ = service
                .library()
                .release_task_input_staging(materialized.object_id, None)
                .await;
            return Err(crate::domain_errors::DomainError::conflict(
                "task item has no captured execution",
            )
            .into());
        };
        if !service
            .db()
            .set_task_item_input_storage_object(
                item.id,
                &execution,
                materialized.object_id,
                &payload,
            )
            .await?
        {
            let _ = service
                .library()
                .release_task_input_staging(materialized.object_id, None)
                .await;
            return Err(crate::domain_errors::DomainError::conflict(
                "task item lease was lost while binding the staged image source",
            )
            .into());
        }
        item.input_storage_object_id = Some(materialized.object_id);
        item.payload = payload;
        let bytes = service
            .library()
            .read_task_input_for_task(group_id, materialized.object_id, item.lease_token)
            .await?;
        if crate::services::library::hash_bytes(&bytes) != materialized.sha256 {
            return Err(crate::domain_errors::DomainError::conflict(
                "the staged image bytes do not match the downloaded submission revision",
            )
            .into());
        }
        return Ok((
            UploadedLibraryFile {
                folder_id: request.folder_id,
                filename,
                media_type,
                bytes,
                declared_sha256: request.declared_sha256.clone(),
                options: request.options.clone(),
                staged_storage_object_id: Some(materialized.object_id),
            },
            materialized.sha256,
        ));
    }
    let encoded = request.content_base64.as_deref().unwrap_or_default();
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|error| anyhow!("invalid image batch content_base64: {error}"))?;
    let bytes = bytes::Bytes::from(bytes);
    let actual = crate::services::library::hash_bytes(&bytes);
    if let Some(expected_sha256) = persisted
        && expected_sha256 != actual
    {
        return Err(crate::domain_errors::DomainError::conflict(
            "the inline image bytes do not match the recorded submission revision",
        )
        .into());
    }
    Ok((
        UploadedLibraryFile {
            folder_id: request.folder_id,
            filename: request.filename.clone(),
            media_type: request.media_type.clone(),
            bytes,
            declared_sha256: request.declared_sha256.clone(),
            options: request.options.clone(),
            staged_storage_object_id: None,
        },
        actual,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::library::ImageOccurrenceRole;

    #[test]
    fn the_default_role_is_body_and_explicit_roles_parse() {
        assert_eq!(
            "body".parse::<ImageOccurrenceRole>().unwrap(),
            ImageOccurrenceRole::Body
        );
        assert!("thumbnail".parse::<ImageOccurrenceRole>().is_err());
    }

    #[test]
    fn a_replayed_image_item_only_continues_for_its_bound_revision() {
        assert_eq!(
            image_revision_state(Some("sha-a"), "sha-a"),
            ImageRevisionState::Bound
        );
        assert_eq!(
            image_revision_state(Some("sha-a"), "sha-b"),
            ImageRevisionState::Superseded
        );
        assert_eq!(
            image_revision_state(None, "sha-a"),
            ImageRevisionState::Missing
        );
        assert!(matches!(
            image_revision_failure("storage", ImageRevisionState::Superseded),
            ProcessResult::Failed {
                retryable: false,
                ..
            }
        ));
        assert!(matches!(
            image_revision_failure("storage", ImageRevisionState::Missing),
            ProcessResult::Failed {
                retryable: false,
                ..
            }
        ));
        assert!(matches!(
            image_revision_failure("storage", ImageRevisionState::Bound),
            ProcessResult::Progressed { next: "embedding" }
        ));
    }

    #[test]
    fn an_image_item_rejects_an_unknown_field_at_the_contract_boundary() {
        let payload = serde_json::json!({
            "filename": "cover.png",
            "media_type": "image/png",
            "content_base64": "aGk=",
            "document_id": 7,
        });
        assert!(serde_json::from_value::<ImageBatchItem>(payload).is_err());
    }

    #[test]
    fn a_whitespace_url_on_a_persisted_staged_inline_replay_stays_inline() {
        use crate::services::tasks::pipeline_progress_input::{
            deserialize_worker_input, normalize_image_source, persisted_image_expected_sha256,
        };

        // The persisted payload after inline staging: `content_base64` is already
        // stripped, the actual SHA is worker-owned, but an old payload may still
        // carry a whitespace-only URL and an extension-resolved blank media type.
        let payload = serde_json::json!({
            "filename": "cover.png",
            "media_type": "",
            "url": "   ",
            "image_source_sha256": "sha-actual",
        });
        let mut request: ImageBatchItem =
            deserialize_worker_input(&payload).expect("the worker input decodes");
        assert_eq!(
            request.url.as_deref(),
            Some("   "),
            "the raw decode still carries the whitespace URL"
        );

        normalize_image_source(&mut request);
        assert!(
            request.url.is_none(),
            "whitespace URL is defensively absent"
        );

        // The actual production replay-source decision must stay inline and not
        // demand URL-resolved metadata or return the missing-metadata conflict.
        let (filename, media_type) =
            staged_replay_identity(&request, None, None).expect("inline replay has an identity");
        assert_eq!(filename, "cover.png");
        assert_eq!(media_type, "");

        // Worker-owned revision metadata survives normalization untouched.
        assert_eq!(
            persisted_image_expected_sha256(&payload),
            Some("sha-actual")
        );
    }

    #[test]
    fn a_real_url_replay_still_uses_persisted_resolved_metadata() {
        let request: ImageBatchItem = serde_json::from_value(serde_json::json!({
            "filename": "cover.png",
            "media_type": "",
            "url": "https://x/cover.png",
        }))
        .expect("url item parses");
        let (filename, media_type) =
            staged_replay_identity(&request, Some("resolved.png"), Some("image/png"))
                .expect("resolved URL replay has an identity");
        assert_eq!(filename, "resolved.png");
        assert_eq!(media_type, "image/png");
    }
}
