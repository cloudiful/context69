use anyhow::Result;
use uuid::Uuid;

use crate::db::Database;
use crate::db::task_file_dedup::{FileDedupDecision, declared_file_id, resolve_file_dedup};

use super::INITIAL_ITEM_STAGE;
use super::execution::{ExecutionClass, FIRST_GENERATION};
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

        // Deduplicate file-ingest items by `file_id`: a file may have at most
        // one active processing item across ingesting task kinds. The
        // resolution validates group ownership, then takes the shared sorted
        // per-file locks before looking for another active task.
        let resolution = resolve_file_dedup(&mut tx, group_id, payloads).await?;
        let retained_indices = match resolution.decide(payloads.len())? {
            FileDedupDecision::Retain(indices) => indices,
            FileDedupDecision::Reuse(active_task_id) => {
                let bound_task_id = match idempotency_key {
                    Some(key) => {
                        bind_idempotency_key(&mut tx, user_id, key, request_hash, active_task_id)
                            .await?
                            .unwrap_or(active_task_id)
                    }
                    None => active_task_id,
                };
                let item_ids =
                    sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", bound_task_id)
                        .fetch_all(&mut *tx)
                        .await?;
                if bound_task_id == active_task_id {
                    tx.commit().await?;
                } else {
                    tx.rollback().await?;
                }
                return Ok((bound_task_id, true, item_ids));
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
        let class = ExecutionClass::for_task_kind(super::execution::task_kind_from_db(kind)?);
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
                class,
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

        if let Some(key) = idempotency_key
            && let Some(winner) =
                bind_idempotency_key(&mut tx, user_id, key, request_hash, task_id).await?
        {
            let item_ids = sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", winner)
                .fetch_all(&mut *tx)
                .await?;
            tx.rollback().await?;
            return Ok((winner, true, item_ids));
        }
        tx.commit().await?;
        Ok((task_id, false, item_ids))
    }
}

/// Binds an accepted submission's idempotency key inside the caller's
/// transaction, and reports the task that already won the key.
///
/// The insert is checked SQL and conflicts without overwriting an accepted
/// hash. The body hash is always compared before the task id: an active file
/// can deduplicate two different requests onto the same task, so a matching
/// task id alone never proves a matching request. A same-hash winner returns its
/// task id so the caller can roll back its own candidate rows; a different hash
/// is a conflict.
async fn bind_idempotency_key(
    connection: &mut sqlx::PgConnection,
    user_id: i64,
    key: &str,
    request_hash: &str,
    task_id: Uuid,
) -> Result<Option<Uuid>> {
    sqlx::query_file!(
        "src/sql/db/tasks/idempotency_put.sql",
        user_id,
        key,
        request_hash,
        task_id
    )
    .execute(&mut *connection)
    .await?;
    let existing = sqlx::query_file_as!(
        StoredIdempotencyKey,
        "src/sql/db/tasks/idempotency_get.sql",
        user_id,
        key
    )
    .fetch_one(&mut *connection)
    .await?;
    if existing.request_hash != request_hash {
        return Err(crate::domain_errors::DomainError::conflict(
            "idempotency key was already used with a different request",
        )
        .into());
    }
    Ok((existing.task_id != task_id).then_some(existing.task_id))
}
