use anyhow::Result;
use uuid::Uuid;

use crate::db::Database;
use crate::db::task_file_dedup::{FileDedupDecision, declared_file_id, resolve_file_dedup};
use crate::db::task_git_dedup::{
    active_git_index_task, declared_git_force_rebuild, declared_git_repository_key,
    git_index_admission_allowed, lock_git_index_admission,
};

use super::INITIAL_ITEM_STAGE;
use super::execution::FIRST_GENERATION;
use super::types::{CreateTaskSubmissionRequest, StoredIdempotencyKey, StoredInputStorageObject};

impl Database {
    pub async fn create_task_submission_with_input_objects(
        &self,
        request: CreateTaskSubmissionRequest<'_>,
    ) -> Result<(Uuid, bool, Vec<Uuid>)> {
        let default_input_ids;
        let input_storage_object_ids: &[Option<Uuid>] = match request.input_storage_object_ids {
            Some(ids) => ids,
            None => {
                default_input_ids = vec![None; request.payloads.len()];
                &default_input_ids
            }
        };
        if request.payloads.len() != input_storage_object_ids.len() {
            return Err(crate::domain_errors::DomainError::invalid_argument(
                "task payload and input object counts do not match",
            )
            .into());
        }
        let task_id = request.task_id;
        let user_id = request.user_id;
        let group_id = request.group_id;
        let kind = request.kind;
        let group_path = request.group_path;
        let source_key = request.source_key;
        let payloads = request.payloads;
        let idempotency_key = request.idempotency_key;
        let request_hash = request.request_hash;
        let mut tx = self.pool().begin().await?;
        if let Some(key) = idempotency_key
            && let Some(existing) = sqlx::query_file_as!(
                StoredIdempotencyKey,
                "src/sql/db/tasks/idempotency_get.sql",
                user_id,
                key
            )
            .fetch_optional(&mut *tx)
            .await?
        {
            if existing.request_hash != request_hash {
                return Err(crate::domain_errors::DomainError::conflict(
                    "idempotency key was already used with a different request",
                )
                .into());
            }
            let item_ids =
                sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", existing.task_id)
                    .fetch_all(&mut *tx)
                    .await?;
            tx.commit().await?;
            return Ok((existing.task_id, true, item_ids));
        }

        // Single-flight per repository for Git index work. The admission lock
        // serializes every submission path; an already active task owns the
        // repository, and the demand marker guarantees a follow-up even when the
        // request arrived while that task was already past its target resolution.
        if kind == "git_index"
            && let Some(repository_key) = declared_git_repository_key(payloads)
        {
            lock_git_index_admission(&mut tx, repository_key).await?;
            let owner_group_id = group_id.ok_or_else(|| {
                crate::domain_errors::DomainError::invalid_argument(
                    "git index tasks require an owning group",
                )
            })?;
            if !git_index_admission_allowed(&mut tx, owner_group_id, repository_key).await? {
                return Err(crate::domain_errors::DomainError::conflict(
                    "git_repository_not_admitting_work",
                )
                .into());
            }
            if let Some(active_task_id) = active_git_index_task(&mut tx, repository_key).await? {
                if declared_git_force_rebuild(payloads) {
                    self.request_git_force_rebuild_in(&mut tx, owner_group_id, repository_key)
                        .await?;
                } else {
                    self.request_git_refresh_in(&mut tx, owner_group_id, repository_key)
                        .await?;
                }
                let item_ids =
                    sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", active_task_id)
                        .fetch_all(&mut *tx)
                        .await?;
                tx.commit().await?;
                return Ok((active_task_id, true, item_ids));
            }
        }

        // Deduplicate file-ingest items by `file_id`: a file may have at most
        // one active processing item across ingesting task kinds. The
        // resolution validates group ownership, then takes the shared sorted
        // per-file locks before looking for another active task.
        let resolution = resolve_file_dedup(&mut tx, group_id, payloads).await?;
        let retained_indices = match resolution.decide(payloads.len())? {
            FileDedupDecision::Retain(indices) => indices,
            FileDedupDecision::Reuse(active_task_id) => {
                let item_ids =
                    sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", active_task_id)
                        .fetch_all(&mut *tx)
                        .await?;
                tx.rollback().await?;
                return Ok((active_task_id, true, item_ids));
            }
        };

        sqlx::query_file!(
            "src/sql/db/tasks/create.sql",
            task_id,
            user_id,
            group_id,
            kind,
            group_path,
            source_key,
            "manual",
            retained_indices.len() as i64
        )
        .fetch_one(&mut *tx)
        .await?;
        for index in &retained_indices {
            let Some(object_id) = input_storage_object_ids[*index] else {
                continue;
            };
            let object = sqlx::query_file_as!(
                StoredInputStorageObject,
                "src/sql/db/tasks/get_input_storage_object.sql",
                object_id
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!(
                    "unknown input storage object {object_id}"
                ))
            })?;
            if Some(object.group_id) != group_id {
                return Err(crate::domain_errors::DomainError::forbidden(
                    "input storage object belongs to another group",
                )
                .into());
            }
            let lock_key = format!("{}:{}", object.group_id, object.sha256);
            sqlx::query_file!("src/sql/db/tasks/lock_input_storage_object.sql", lock_key)
                .execute(&mut *tx)
                .await?;
            let exists = sqlx::query_file_as!(
                StoredInputStorageObject,
                "src/sql/db/tasks/get_input_storage_object.sql",
                object_id
            )
            .fetch_optional(&mut *tx)
            .await?;
            if exists.is_none() {
                return Err(crate::domain_errors::DomainError::not_found(format!(
                    "input storage object {object_id} disappeared"
                ))
                .into());
            }
            sqlx::query_file!(
                "src/sql/db/tasks/refresh_input_storage_object.sql",
                object_id
            )
            .execute(&mut *tx)
            .await?;
        }
        let mut item_ids = Vec::with_capacity(retained_indices.len());
        let queue = self.execution_queue().await?;
        for (ordinal, index) in retained_indices.iter().enumerate() {
            let payload = &payloads[*index];
            let item_id = Uuid::new_v4();
            item_ids.push(item_id);
            // The segment of the item is produced in the same transaction as the
            // item, so a visible item always has the segment that runs it.
            let segment = super::execution::produce_segment(
                queue,
                &mut tx,
                super::execution::SegmentPayload::new(item_id, task_id, FIRST_GENERATION),
            )
            .await?;
            sqlx::query_file!(
                "src/sql/db/tasks/insert_item.sql",
                item_id,
                task_id,
                ordinal as i32,
                payload,
                INITIAL_ITEM_STAGE,
                declared_file_id(payload),
                input_storage_object_ids[*index],
                segment.as_uuid(),
                FIRST_GENERATION
            )
            .execute(&mut *tx)
            .await?;
        }

        if let Some(key) = idempotency_key {
            sqlx::query_file!(
                "src/sql/db/tasks/idempotency_put.sql",
                user_id,
                key,
                request_hash,
                task_id
            )
            .execute(&mut *tx)
            .await?;
            let existing = sqlx::query_file_as!(
                StoredIdempotencyKey,
                "src/sql/db/tasks/idempotency_get.sql",
                user_id,
                key
            )
            .fetch_one(&mut *tx)
            .await?;
            if existing.task_id != task_id {
                if existing.request_hash != request_hash {
                    return Err(crate::domain_errors::DomainError::conflict(
                        "idempotency key was already used with a different request",
                    )
                    .into());
                }
                let item_ids =
                    sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", existing.task_id)
                        .fetch_all(&mut *tx)
                        .await?;
                tx.rollback().await?;
                return Ok((existing.task_id, true, item_ids));
            }
        }
        tx.commit().await?;
        Ok((task_id, false, item_ids))
    }
}
