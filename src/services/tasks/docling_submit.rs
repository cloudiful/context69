use anyhow::Result;

use super::TaskService;
use super::item_processors::ProcessResult;

/// The fixed, sanitized reason of a fail-closed Docling submission.
///
/// It never carries an endpoint, a credential, a response body or a path.
pub(super) const UNCERTAIN_SUBMISSION_REASON: &str =
    "the Docling dispatch outcome is unknown; acknowledgement is required";

/// Runs one durable Docling submission segment of a claimed item (issue 1172
/// P5A/P6B0). It submits once, adopts one unresolved row, or parks on the
/// existing durable intent; it never waits inline for the whole remote job.
///
/// The sequence is fail-closed: an unresolved row is loaded first and either
/// adopted (known remote id) or parked as uncertain with zero HTTP; a fresh
/// submission persists its frozen intent before preparation, marks the
/// dispatch attempted before `.send()`, and turns every post-dispatch failure
/// into a durable unknown outcome instead of a resend.
pub(super) async fn run_docling_submission_segment(
    service: &TaskService,
    task_id: uuid::Uuid,
    item: &mut crate::db::ClaimedItem,
    file_id: uuid::Uuid,
) -> Result<ProcessResult> {
    run_docling_submission_segment_with_view(service, task_id, item, file_id, None).await
}

pub(super) async fn run_docling_submission_segment_with_view(
    service: &TaskService,
    task_id: uuid::Uuid,
    item: &mut crate::db::ClaimedItem,
    file_id: uuid::Uuid,
    view: Option<&crate::services::library::ReplacementFileAttributes>,
) -> Result<ProcessResult> {
    let Some(execution) = item.execution_context() else {
        return Ok(ProcessResult::Failed {
            stage: "docling".to_string(),
            message: "the claimed item carries no captured execution".to_string(),
            retryable: false,
        });
    };
    let fence = crate::db::DoclingSubmissionFence {
        task_id,
        item_id: item.id,
        lease_token: item.lease_token,
        execution_task_id: execution.task_id(),
        generation: execution.generation,
    };
    let source = crate::services::library::TaskSourceRef::from_input(item.input_storage_object_id);

    if let Some(job) = service
        .db()
        .get_unresolved_docling_remote_job_for_item(item.id)
        .await?
    {
        return resume_unresolved_submission(service, item, file_id, job, view).await;
    }

    let plan = match service
        .library()
        .prepare_docling_submission(file_id, source, view)
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(if error.retryable {
                super::item_processors::waiting_for_error(item, error)
            } else {
                ProcessResult::Failed {
                    stage: error.stage,
                    message: error.message,
                    retryable: false,
                }
            });
        }
    };

    let now = chrono::Utc::now();
    let intent = service
        .db()
        .begin_docling_submission(crate::db::DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: plan.endpoint_base_url(),
            submission_identity: &plan.identity,
            deadline_at: now + plan.deadline_delay,
        })
        .await?;
    let Some(job) = intent else {
        // The unique unresolved index or the item fence refused the insert: a
        // concurrent submission already owns this item, so it is reread and
        // never sent a second time.
        if let Some(job) = service
            .db()
            .get_unresolved_docling_remote_job_for_item(item.id)
            .await?
        {
            return resume_unresolved_submission(service, item, file_id, job, view).await;
        }
        return Ok(ProcessResult::Failed {
            stage: "docling".to_string(),
            message: "the Docling submission intent could not be persisted".to_string(),
            retryable: true,
        });
    };

    let prepared = match service
        .library()
        .build_prepared_docling_submission(&plan, item.lease_token)
        .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = service
                .db()
                .mark_docling_preparation_failed(
                    job.id,
                    &fence,
                    "docling preparation failed before dispatch",
                )
                .await;
            return Ok(ProcessResult::Failed {
                stage: error.stage,
                message: error.message,
                retryable: false,
            });
        }
    };

    let marked = service
        .db()
        .mark_docling_dispatch_attempted(job.id, &fence)
        .await?;
    if !marked {
        // The captured execution is gone, so the durable intent can no longer be
        // resolved by this worker: a submitting row is always a conservative
        // unknown outcome, never an automatic retry.
        return Ok(uncertain_result(job.id));
    }
    let next_poll_at = now + plan.next_poll_delay;
    match service
        .library()
        .dispatch_docling_submission(prepared)
        .await
    {
        Ok(remote_task_id) => {
            let confirmed = service
                .db()
                .confirm_docling_submission(job.id, &fence, &remote_task_id, next_poll_at)
                .await?;
            match confirmed {
                Some(confirmed) => {
                    super::docling_poll::run_docling_stage_probe(
                        service.db(),
                        service.library(),
                        item,
                        file_id,
                        confirmed,
                        view,
                    )
                    .await
                }
                // The acknowledgement was lost: the durable row decides.
                None => {
                    match service
                        .db()
                        .get_unresolved_docling_remote_job_for_item(item.id)
                        .await?
                    {
                        Some(job) => {
                            resume_unresolved_submission(service, item, file_id, job, view).await
                        }
                        None => Ok(uncertain_result(job.id)),
                    }
                }
            }
        }
        Err(_) => Ok(uncertain_result(job.id)),
    }
}

/// The typed unknown outcome that never waits inline and never resubmits.
fn uncertain_result(job_id: uuid::Uuid) -> ProcessResult {
    ProcessResult::DoclingSubmissionUncertain {
        job_id,
        reason: UNCERTAIN_SUBMISSION_REASON.to_string(),
    }
}

/// Resumes one already-persisted submission without sending anything.
///
/// A known `pending`/`running` row is adopted by the P4 poll path. Every other
/// unresolved state is conservative: the dispatch may have been invoked, so
/// the item parks as uncertain with zero HTTP.
async fn resume_unresolved_submission(
    service: &TaskService,
    item: &mut crate::db::ClaimedItem,
    file_id: uuid::Uuid,
    job: crate::db::StoredDoclingRemoteJob,
    view: Option<&crate::services::library::ReplacementFileAttributes>,
) -> Result<ProcessResult> {
    match job.status.as_str() {
        "pending" | "running" => {
            if job.remote_task_id.is_none() {
                return Ok(ProcessResult::Failed {
                    stage: "docling".to_string(),
                    message: "the active Docling job carries no remote id".to_string(),
                    retryable: false,
                });
            }
            super::docling_poll::run_docling_stage_probe(
                service.db(),
                service.library(),
                item,
                file_id,
                job,
                view,
            )
            .await
        }
        _ => Ok(uncertain_result(job.id)),
    }
}
