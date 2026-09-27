use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::{FileBatchItem, TaskKind, TaskRef};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{db::CreateTaskSubmissionRequest, domain_errors::DomainError};

use super::{TaskService, TaskSubmission};

impl TaskService {
    pub async fn submit(&self, request: TaskSubmission) -> Result<TaskRef> {
        if request.payloads.is_empty() {
            return Err(
                DomainError::invalid_argument("a task must contain at least one item").into(),
            );
        }
        let request_hash = hash_payload(&request);
        let mut payloads = request.payloads.clone();
        let mut input_storage_object_ids = if request.input_storage_object_ids.is_empty() {
            vec![None; payloads.len()]
        } else {
            request.input_storage_object_ids.clone()
        };
        if input_storage_object_ids.len() != payloads.len() {
            return Err(DomainError::invalid_argument(
                "task payload and input object counts do not match",
            )
            .into());
        }
        let mut newly_staged_object_ids = Vec::new();
        if request.kind == TaskKind::FileBatch {
            let group_id = request
                .group_id
                .ok_or_else(|| DomainError::invalid_argument("file tasks require group_id"))?;
            let result = async {
                for (index, payload) in payloads.iter_mut().enumerate() {
                    if payload.get("file_id").is_some() || payload.get("content_base64").is_none() {
                        continue;
                    }
                    let file: FileBatchItem =
                        serde_json::from_value(payload.clone()).map_err(|error| {
                            DomainError::invalid_argument(format!(
                                "invalid file batch item: {error}"
                            ))
                        })?;
                    let bytes = STANDARD
                        .decode(file.content_base64.trim())
                        .map_err(|error| {
                            DomainError::invalid_argument(format!(
                                "invalid file batch content_base64: {error}"
                            ))
                        })?;
                    let object_id = self
                        .library
                        .stage_file_for_task_input(
                            group_id,
                            crate::services::library::UploadedLibraryFile {
                                folder_id: file.folder_id,
                                filename: file.filename,
                                media_type: file.media_type,
                                bytes: bytes.into(),
                                declared_sha256: file.declared_sha256,
                                options: file.options.clone(),
                                staged_storage_object_id: None,
                            },
                        )
                        .await?;
                    input_storage_object_ids[index] = Some(object_id);
                    newly_staged_object_ids.push(object_id);
                    if let Some(object) = payload.as_object_mut() {
                        object.remove("content_base64");
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.release_staged_input_objects(&newly_staged_object_ids)
                    .await;
                return Err(error);
            }
        }
        let key = request
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let task_id = Uuid::new_v4();
        let submission = self
            .db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: request.user_id,
                group_id: request.group_id,
                kind: request.kind.as_str(),
                group_path: request.group_path.as_deref(),
                source_key: request.source_key.as_deref(),
                payloads: &payloads,
                input_storage_object_ids: Some(&input_storage_object_ids),
                idempotency_key: key,
                request_hash: &request_hash,
            })
            .await;
        let (task_id, reused, item_ids) = match submission {
            Ok(value) => value,
            Err(error) => {
                self.release_staged_input_objects(&newly_staged_object_ids)
                    .await;
                return Err(error);
            }
        };
        if reused {
            self.release_staged_input_objects(&newly_staged_object_ids)
                .await;
        }
        if !reused {
            self.notify_dispatch();
        }
        Ok(TaskRef { task_id, item_ids })
    }

    async fn release_staged_input_objects(&self, object_ids: &[Uuid]) {
        for &object_id in object_ids {
            if let Err(error) = self
                .library
                .release_task_input_staging(object_id, None)
                .await
            {
                tracing::warn!(%object_id, %error, "failed to release staged task input after submission failure");
            }
        }
    }
}

fn hash_payload(request: &TaskSubmission) -> String {
    let bytes = serde_json::to_vec(&(
        &request.kind,
        &request.group_id,
        &request.group_path,
        &request.source_key,
        &request.payloads,
    ))
    .expect("task payloads are serializable");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
