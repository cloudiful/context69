use anyhow::Result;

use super::TaskService;
use super::item_processors::ProcessResult;

/// Submits a Docling file and blocks the worker on the conversion (issue 650
/// P3): submit, status polling, and result fetching are one blocking flow
/// inside the admitted parent slot. The item lease stays alive under the
/// runtime heartbeat (and the parent slot under the recovery tick) for the
/// whole wait; nothing parks, and no sweep requeues.
///
/// A crash between the submit and the terminal commit is resumed, not
/// resubmitted: an active remote row for the item short-circuits straight
/// into the poll loop, fenced by the partial unique active-row index. On
/// terminal success the committed sections are written back into the same
/// in-memory snapshot, so the same claim advances to `embedding` without
/// re-entering Docling.
pub(super) async fn submit_and_await_blocking(
    service: &TaskService,
    task_id: uuid::Uuid,
    item: &mut crate::db::ClaimedItem,
    file_id: uuid::Uuid,
) -> Result<ProcessResult> {
    if let Some(active) = service
        .db()
        .get_active_docling_remote_job_for_item(item.id)
        .await?
    {
        // Crash/restart resume: a previous worker submitted but never
        // committed. Adopt the remote id instead of submitting a duplicate.
        return super::docling_poll::run_docling_stage_blocking(
            service.db(),
            service.library(),
            item,
            file_id,
            active,
        )
        .await;
    }
    let submit = match service
        .library()
        .submit_docling_remote_for_task(file_id, item.lease_token, task_id)
        .await
    {
        Ok(submit) => submit,
        Err(error) => {
            // Retryable submit failures (service down, transport) return a
            // backoff wait that the blocking driver sleeps inline keeping
            // the lease, then re-drives into a fresh submit.
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
        Ok(job) => {
            super::docling_poll::run_docling_stage_blocking(
                service.db(),
                service.library(),
                item,
                file_id,
                job,
            )
            .await
        }
        Err(error) => {
            // Lost race with a concurrent submit (unique active/remote): adopt
            // the winner instead of failing the item.
            let message = error.to_string().to_ascii_lowercase();
            if (message.contains("duplicate")
                || message.contains("unique")
                || message.contains("already exists"))
                && let Some(active) = service
                    .db()
                    .get_active_docling_remote_job_for_item(item.id)
                    .await?
            {
                return super::docling_poll::run_docling_stage_blocking(
                    service.db(),
                    service.library(),
                    item,
                    file_id,
                    active,
                )
                .await;
            }
            // Transient persist failure after a successful submit: the
            // conversion is already running server-side under an id nobody
            // polls yet. Retry the insert a bounded number of times before
            // giving up; on final failure the item retries and submits a new
            // remote task, leaving the orphaned conversion to run out
            // server-side unfetched.
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
                            return super::docling_poll::run_docling_stage_blocking(
                                service.db(),
                                service.library(),
                                item,
                                file_id,
                                job,
                            )
                            .await;
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

/// Unique-violation inserts are handled by adopting the winning row, never
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
