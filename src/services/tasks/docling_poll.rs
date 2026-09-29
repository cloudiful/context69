use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::{Database, StoredDoclingRemoteJob};
use crate::services::library::{LibraryDependency, LibraryService, UnifiedIngestError};

use super::docling_finalize::{finalize_docling_failure, finalize_docling_success};
use super::docling_heartbeat::{SWEEP_LEASE_SECS, with_remote_lease_heartbeat};

const SWEEP_MIN_POLL_SECS: i64 = 2;
const SWEEP_MAX_BACKOFF_SECS: i64 = 30;

pub(crate) enum PollOutcome {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Pending,
}

fn backoff_secs(attempt: i32) -> i64 {
    match attempt {
        i32::MIN..=0 => SWEEP_MIN_POLL_SECS,
        1 => 4,
        2 => 8,
        3 => 15,
        _ => SWEEP_MAX_BACKOFF_SECS,
    }
}

fn next_poll_at(attempt: i32) -> DateTime<Utc> {
    Utc::now() + chrono::Duration::seconds(backoff_secs(attempt))
}

fn is_expired(job: &StoredDoclingRemoteJob) -> bool {
    job.deadline_at
        .is_some_and(|deadline| deadline <= Utc::now())
}

async fn record_pending(
    db: &Database,
    job: &StoredDoclingRemoteJob,
    remote_status: Option<&str>,
    last_error: Option<&str>,
) -> Result<()> {
    db.record_docling_remote_job_pending(
        job.id,
        job.lease_token
            .ok_or_else(|| anyhow::anyhow!("sweep job missing lease"))?,
        remote_status,
        next_poll_at(job.attempt_count),
        last_error,
    )
    .await?;
    Ok(())
}

/// Records a Docling error against the Docling dependency gate. The gate
/// itself filters: transient/network errors open the gate, configuration
/// errors pin it, remote conversion failures are ignored.
async fn note_docling_error(library: &LibraryService, error: &UnifiedIngestError) {
    library
        .note_dependency_failure(
            LibraryDependency::Docling,
            &anyhow::anyhow!(error.message.clone()),
        )
        .await;
}

async fn timeout_job(db: &Database, job: &StoredDoclingRemoteJob) -> Result<PollOutcome> {
    let message = "remote job deadline exceeded".to_string();
    finalize_docling_failure(
        db,
        job,
        "timed_out",
        job.remote_status.as_deref(),
        Some(&message),
        "docling",
        &message,
    )
    .await?;
    Ok(PollOutcome::TimedOut)
}

async fn fail_job(
    db: &Database,
    library: &LibraryService,
    job: &StoredDoclingRemoteJob,
    remote_status: Option<&str>,
    message: &str,
) -> Result<PollOutcome> {
    note_docling_error(
        library,
        &UnifiedIngestError {
            stage: "docling".to_string(),
            dependency_key: Some("docling".to_string()),
            retryable: false,
            message: message.to_string(),
        },
    )
    .await;
    finalize_docling_failure(
        db,
        job,
        "failed",
        remote_status,
        Some(message),
        "docling",
        message,
    )
    .await?;
    Ok(PollOutcome::Failed)
}

pub(crate) async fn poll_one(
    db: Database,
    library: LibraryService,
    job: StoredDoclingRemoteJob,
) -> Result<PollOutcome> {
    let Some(item) = db.get_docling_sweep_item(job.item_id).await? else {
        db.cancel_active_docling_remote_job_for_item(job.item_id, Some("owning item disappeared"))
            .await?;
        return Ok(PollOutcome::Cancelled);
    };
    if item.status != "waiting" || item.waiting_reason.as_deref() != Some("docling") {
        db.cancel_active_docling_remote_job_for_item(
            job.item_id,
            Some("owning item left docling wait"),
        )
        .await?;
        return Ok(PollOutcome::Cancelled);
    }
    if is_expired(&job) {
        return timeout_job(&db, &job).await;
    }
    // The lease is renewed underneath the long poll: a lost lease means a
    // concurrent timeout/cancel finalization owns the row now, so the
    // in-flight request is cancelled here and its result is never written.
    let status = match with_remote_lease_heartbeat(
        &db,
        &job,
        SWEEP_LEASE_SECS,
        library.poll_docling_remote(&job.remote_task_id),
    )
    .await?
    {
        None => return Ok(PollOutcome::Cancelled),
        Some(Ok(status)) => status,
        Some(Err(error)) => {
            note_docling_error(&library, &error).await;
            if !error.retryable {
                let message = error.message.clone();
                finalize_docling_failure(
                    &db,
                    &job,
                    "failed",
                    None,
                    Some(&message),
                    "docling",
                    &message,
                )
                .await?;
                return Ok(PollOutcome::Failed);
            }
            let message = error.message.clone();
            record_pending(&db, &job, job.remote_status.as_deref(), Some(&message)).await?;
            return Ok(PollOutcome::Pending);
        }
    };
    let status_name = format!("{:?}", status.task_status).to_ascii_lowercase();
    if !status.task_status.is_terminal() {
        record_pending(&db, &job, Some(&status_name), None).await?;
        return Ok(PollOutcome::Pending);
    }
    if !status.task_status.is_successful() {
        let message = status
            .error_message
            .clone()
            .or_else(|| {
                status
                    .failure
                    .as_ref()
                    .map(|failure| failure.message.clone())
            })
            .unwrap_or_else(|| format!("docling task {} failed", job.remote_task_id));
        return fail_job(&db, &library, &job, Some(&status_name), &message).await;
    }
    let Some(file_id) = item.file_id else {
        return fail_job(
            &db,
            &library,
            &job,
            Some(&status_name),
            "parked docling item has no file",
        )
        .await;
    };
    // A slow poll may have crossed the deadline, and the expiry handler may
    // own the row now: recheck before fetching and committing success.
    if is_expired(&job) {
        return timeout_job(&db, &job).await;
    }
    let converted = match with_remote_lease_heartbeat(
        &db,
        &job,
        SWEEP_LEASE_SECS,
        library.fetch_docling_remote(file_id, &job.remote_task_id),
    )
    .await?
    {
        None => return Ok(PollOutcome::Cancelled),
        Some(Ok(converted)) => converted,
        Some(Err(error)) => {
            note_docling_error(&library, &error).await;
            if !error.retryable {
                let message = error.message.clone();
                return fail_job(&db, &library, &job, Some(&status_name), &message).await;
            }
            let message = error.message.clone();
            record_pending(&db, &job, Some(&status_name), Some(&message)).await?;
            return Ok(PollOutcome::Pending);
        }
    };
    if is_expired(&job) {
        return timeout_job(&db, &job).await;
    }
    if finalize_docling_success(&db, &library, &job, &status_name, converted).await? {
        library
            .note_dependency_success(LibraryDependency::Docling, Uuid::nil())
            .await;
        Ok(PollOutcome::Succeeded)
    } else {
        Ok(PollOutcome::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::backoff_secs;

    #[test]
    fn poll_backoff_is_bounded_and_starts_at_two_seconds() {
        for (attempt, expected) in [(0, 2), (1, 4), (2, 8), (3, 15), (4, 30), (100, 30)] {
            assert_eq!(backoff_secs(attempt), expected);
        }
    }
}
