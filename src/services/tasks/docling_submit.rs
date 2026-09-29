use anyhow::Result;
use chrono::Utc;

use super::TaskService;
use super::item_processors::ProcessResult;

/// Submits a Docling file for durable polling and parks the worker.
///
/// Returns `Waiting` with `waiting_reason='docling'` so the dispatcher
/// exclusion (`claim_items.sql` NOT EXISTS) keeps the item away from normal
/// workers until the sweep requeues it. Capacity is released as soon as this
/// returns; the Docling permit is held only for the submit POST inside the
/// library call.
pub(super) async fn submit_and_park(
    service: &TaskService,
    task_id: uuid::Uuid,
    item: &crate::db::ClaimedItem,
    file_id: uuid::Uuid,
) -> Result<ProcessResult> {
    if let Some(active) = service
        .db()
        .get_active_docling_remote_job_for_item(item.id)
        .await?
    {
        return Ok(ProcessResult::Waiting {
            reason: "docling".to_string(),
            dependency_key: Some("docling".to_string()),
            next_attempt_at: active.next_poll_at.min(
                active
                    .deadline_at
                    .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(1)),
            ),
            message: Some(format!("docling remote {} pending", active.remote_task_id)),
        });
    }
    let submit = match service
        .library()
        .submit_docling_remote_for_task(file_id, item.lease_token, task_id)
        .await
    {
        Ok(submit) => submit,
        Err(error) => {
            if error.retryable {
                return Ok(super::item_processors::waiting_for_error(item, error));
            }
            return Ok(ProcessResult::Failed {
                stage: error.stage,
                message: error.message,
                retryable: false,
            });
        }
    };
    match service
        .db()
        .create_docling_remote_job(
            task_id,
            item.id,
            &submit.remote_task_id,
            Some("submitted"),
            Some(submit.next_poll_at),
            Some(submit.deadline_at),
        )
        .await
    {
        Ok(job) => Ok(ProcessResult::Waiting {
            reason: "docling".to_string(),
            dependency_key: Some("docling".to_string()),
            next_attempt_at: job.next_poll_at,
            message: Some(format!("docling remote {} submitted", job.remote_task_id)),
        }),
        Err(error) => {
            // Lost race with a concurrent submit (unique active/remote): park
            // on the winner instead of failing the item.
            let message = error.to_string().to_ascii_lowercase();
            if (message.contains("duplicate")
                || message.contains("unique")
                || message.contains("already exists"))
                && let Some(active) = service
                    .db()
                    .get_active_docling_remote_job_for_item(item.id)
                    .await?
            {
                return Ok(ProcessResult::Waiting {
                    reason: "docling".to_string(),
                    dependency_key: Some("docling".to_string()),
                    next_attempt_at: active.next_poll_at,
                    message: Some(format!("docling remote {} pending", active.remote_task_id)),
                });
            }
            // Transient persist failure after a successful submit: the
            // conversion is already running server-side under an id nobody
            // polls yet. Retry the insert a bounded number of times before
            // giving up; on final failure the item retries and submits a new
            // remote task, leaving the orphaned conversion to run out
            // server-side unfetched (see Remaining in the phase note).
            if is_transient_persist_error(&message) {
                for backoff_ms in [100, 250] {
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    match service
                        .db()
                        .create_docling_remote_job(
                            task_id,
                            item.id,
                            &submit.remote_task_id,
                            Some("submitted"),
                            Some(submit.next_poll_at),
                            Some(submit.deadline_at),
                        )
                        .await
                    {
                        Ok(job) => {
                            return Ok(ProcessResult::Waiting {
                                reason: "docling".to_string(),
                                dependency_key: Some("docling".to_string()),
                                next_attempt_at: job.next_poll_at,
                                message: Some(format!(
                                    "docling remote {} submitted",
                                    job.remote_task_id
                                )),
                            });
                        }
                        Err(retry_error) => {
                            let retry_message = retry_error.to_string().to_ascii_lowercase();
                            if !is_transient_persist_error(&retry_message) {
                                break;
                            }
                        }
                    }
                }
            }
            Ok(ProcessResult::Failed {
                stage: "docling".to_string(),
                message: format!("failed to persist docling remote job: {error}"),
                retryable: true,
            })
        }
    }
}

/// Unique-violation inserts are handled by parking on the winning row, never
/// retried here. Everything else from the persist step is treated as
/// transient (connection/read timeouts, serialization failures) and gets the
/// bounded insert retry above.
fn is_transient_persist_error(lower_message: &str) -> bool {
    !(lower_message.contains("duplicate")
        || lower_message.contains("unique")
        || lower_message.contains("already exists")
        || lower_message.contains("foreign key")
        || lower_message.contains("violates"))
}
