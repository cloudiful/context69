use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::db::{Database, StoredDoclingRemoteJob};
use crate::docling::{ConvertedDocument, TaskStatus, TaskStatusObservation};
use crate::domain_errors::DomainError;
use crate::services::library::{LibraryDependency, LibraryService, UnifiedIngestError};

use super::docling_finalize::payload_with_sections;
use super::item_processors::ProcessResult;

/// Cancel-responsiveness bound for one poll-interval sleep chunk. Every chunk
/// re-reads the owned item row, so a task cancel aborts the blocking wait
/// within this long (plus the in-flight poll request, which is bounded by
/// the client's `?wait=30` long poll).
const POLL_SLEEP_CHUNK_SECS: i64 = 10;

/// Remote-conversion operations the blocking Docling stage needs. Implemented
/// by [`LibraryService`]; tests substitute a scripted fake so the real loop
/// (including the in-memory snapshot commit) runs without a live converter.
#[async_trait::async_trait]
pub(super) trait DoclingRemoteOps: Send + Sync {
    /// Polls one remote task at the service root the job was submitted to.
    async fn poll_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<TaskStatusObservation, UnifiedIngestError>;
    /// Fetches the converted document at the service root the job was
    /// submitted to.
    async fn fetch_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<ConvertedDocument, UnifiedIngestError>;
    async fn remote_sections(
        &self,
        file_id: Uuid,
        converted: ConvertedDocument,
    ) -> Result<Value, UnifiedIngestError>;
    /// The converted sections under an optional projected file view.
    ///
    /// The default keeps every existing implementation on its own parser; the
    /// library service overrides it so an ordinary replacement parses with the
    /// announced filename and media type.
    async fn remote_sections_with_view(
        &self,
        file_id: Uuid,
        converted: ConvertedDocument,
        view: Option<&crate::services::library::ReplacementFileAttributes>,
    ) -> Result<Value, UnifiedIngestError> {
        let _ = view;
        self.remote_sections(file_id, converted).await
    }
    async fn note_remote_failure(&self, error: &UnifiedIngestError);
    async fn note_remote_success(&self);
    /// Externalize the just-committed inline sections for a group-owned item.
    ///
    /// The inline commit has already happened, so an error here leaves a valid
    /// inline checkpoint and never resubmits the remote conversion.
    async fn externalize_sections(
        &self,
        item: &mut crate::db::ClaimedItem,
        sections: &Value,
    ) -> Result<()>;
}

#[async_trait::async_trait]
impl DoclingRemoteOps for LibraryService {
    async fn poll_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<TaskStatusObservation, UnifiedIngestError> {
        self.poll_docling_remote(endpoint_base_url, remote_task_id)
            .await
    }

    async fn fetch_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<ConvertedDocument, UnifiedIngestError> {
        self.fetch_docling_remote(endpoint_base_url, remote_task_id)
            .await
    }

    async fn remote_sections(
        &self,
        file_id: Uuid,
        converted: ConvertedDocument,
    ) -> Result<Value, UnifiedIngestError> {
        self.sections_for_remote_result(file_id, converted).await
    }

    async fn remote_sections_with_view(
        &self,
        file_id: Uuid,
        converted: ConvertedDocument,
        view: Option<&crate::services::library::ReplacementFileAttributes>,
    ) -> Result<Value, UnifiedIngestError> {
        self.sections_for_remote_result_with_view(file_id, converted, view)
            .await
    }

    async fn note_remote_failure(&self, error: &UnifiedIngestError) {
        self.note_dependency_failure(
            LibraryDependency::Docling,
            &anyhow::anyhow!(error.message.clone()),
        )
        .await;
    }

    async fn note_remote_success(&self) {
        self.note_dependency_success(LibraryDependency::Docling, Uuid::nil())
            .await;
    }

    async fn externalize_sections(
        &self,
        item: &mut crate::db::ClaimedItem,
        sections: &Value,
    ) -> Result<()> {
        self.externalize_task_item_sections(item, sections).await
    }
}

fn backoff_secs(attempt: i32) -> i64 {
    match attempt {
        i32::MIN..=0 => 2,
        1 => 4,
        2 => 8,
        3 => 15,
        _ => 30,
    }
}

fn is_expired(deadline_at: Option<DateTime<Utc>>) -> bool {
    deadline_at.is_some_and(|deadline| deadline <= Utc::now())
}

fn lease_lost() -> ProcessResult {
    ProcessResult::Failed {
        stage: "docling".to_string(),
        message: "task item lease was lost during the docling wait".to_string(),
        retryable: false,
    }
}

enum OwnerState {
    Ours,
    Gone,
    Stolen,
}

/// Checks whether the blocking worker still owns its item: `Gone` means the
/// item left `running` (cancelled, failed, or requeued elsewhere); recovery
/// alone fences confirmed terminal owners. `Stolen` means another worker holds the
/// item lease (crash reclaim) and this worker must write nothing at all.
async fn owner_state(db: &Database, item_id: Uuid, lease_token: Uuid) -> Result<OwnerState> {
    let work = db.get_docling_work_item(item_id).await?;
    Ok(match work {
        None => OwnerState::Gone,
        Some(work) if work.status != "running" => OwnerState::Gone,
        Some(work) if work.lease_token != Some(lease_token) => OwnerState::Stolen,
        Some(_) => OwnerState::Ours,
    })
}

/// Sleeps `secs` in bounded chunks while the worker still owns its item.
/// Returns `false` when ownership is lost mid-sleep; the caller commits
/// nothing in that case and lets the fenced finish observe the loss.
async fn sleep_owned(db: &Database, item: &crate::db::ClaimedItem, secs: i64) -> Result<bool> {
    let mut remaining = secs.max(0);
    while remaining > 0 {
        let chunk = remaining.min(POLL_SLEEP_CHUNK_SECS);
        tokio::time::sleep(Duration::from_secs(chunk as u64)).await;
        remaining -= chunk;
        if !matches!(
            owner_state(db, item.id, item.lease_token).await?,
            OwnerState::Ours
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Blocks the worker on one submitted remote conversion until it resolves.
///
/// The item lease stays alive under the runtime heartbeat (and the admitted
/// parent slot under the recovery tick) for the whole wait, so the parent
/// task is never released mid-conversion. Outcomes:
/// - terminal success: sections are committed atomically and the same
///   in-memory snapshot is updated, then the pipeline advances to
///   `embedding` without re-entering Docling;
/// - terminal remote failure, non-retryable transport/parse error, or
///   deadline expiry: the item fails without writing the remote row, so a
///   crash before the item commit re-observes instead of resubmitting, and
///   the recovery pass cancels the orphaned row once the item is terminal
///   (within one recovery tick);
/// - lost ownership (cancel, reclaim): write nothing; the fenced item finish
///   observes the loss and terminal-owner recovery handles cleanup.
pub(super) async fn run_docling_stage_blocking(
    db: &Database,
    ops: &(impl DoclingRemoteOps + '_),
    item: &mut crate::db::ClaimedItem,
    file_id: Uuid,
    job: StoredDoclingRemoteJob,
    view: Option<&crate::services::library::ReplacementFileAttributes>,
) -> Result<ProcessResult> {
    let mut attempt: i32 = 0;
    // The captured root is the endpoint the job was submitted to: a settings
    // change never moves a persisted job to another service.
    let Some(endpoint_base_url) = job.endpoint_base_url.as_deref() else {
        return Ok(ProcessResult::Failed {
            stage: "docling".to_string(),
            message: "the remote job has no captured docling endpoint".to_string(),
            retryable: false,
        });
    };
    loop {
        if let Some(execution) = item.execution_context()
            && db.execution_pause_requested(&execution).await?
        {
            return Ok(ProcessResult::Paused);
        }
        match owner_state(db, item.id, item.lease_token).await? {
            OwnerState::Gone => {
                return Ok(ProcessResult::Failed {
                    stage: "docling".to_string(),
                    message: "owning item left the running worker".to_string(),
                    retryable: false,
                });
            }
            OwnerState::Stolen => return Ok(lease_lost()),
            OwnerState::Ours => {}
        }
        if is_expired(job.deadline_at) {
            return Ok(ProcessResult::Failed {
                stage: "docling".to_string(),
                message: "remote job deadline exceeded".to_string(),
                retryable: false,
            });
        }
        let status = match ops
            .poll_remote(endpoint_base_url, &job.remote_task_id)
            .await
        {
            Ok(status) => status,
            Err(error) => {
                ops.note_remote_failure(&error).await;
                if !error.retryable {
                    return Ok(ProcessResult::Failed {
                        stage: "docling".to_string(),
                        message: error.message.clone(),
                        retryable: false,
                    });
                }
                attempt += 1;
                if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                    return Ok(lease_lost());
                }
                continue;
            }
        };
        let status_name = status.task_status.wire_name().to_string();
        // Only a changed observation is progress: the Docling client exposes
        // only a task status (no page counts), and a repeated long-poll result
        // is not new work, so it must not move the progress timestamp.
        super::pipeline_progress::record_remote_status(db, item, &status_name).await?;
        if !status.task_status.is_terminal() {
            attempt += 1;
            if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                return Ok(lease_lost());
            }
            continue;
        }
        if !status.task_status.is_successful() {
            let message = status
                .failure_message()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if status.task_status == TaskStatus::Cancelled {
                        format!("docling task {} was cancelled", job.remote_task_id)
                    } else {
                        format!("docling task {} failed", job.remote_task_id)
                    }
                });
            ops.note_remote_failure(&UnifiedIngestError {
                stage: "docling".to_string(),
                dependency_key: Some("docling".to_string()),
                retryable: false,
                message: message.clone(),
            })
            .await;
            return Ok(ProcessResult::Failed {
                stage: "docling".to_string(),
                message,
                retryable: false,
            });
        }
        let converted = match ops
            .fetch_remote(endpoint_base_url, &job.remote_task_id)
            .await
        {
            Ok(converted) => converted,
            Err(error) => {
                ops.note_remote_failure(&error).await;
                if !error.retryable {
                    return Ok(ProcessResult::Failed {
                        stage: "docling".to_string(),
                        message: error.message.clone(),
                        retryable: false,
                    });
                }
                attempt += 1;
                if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                    return Ok(lease_lost());
                }
                continue;
            }
        };
        // The fetch took time: re-read the owned row for the fresh payload
        // and re-check ownership before committing anything.
        let work = db.get_docling_work_item(item.id).await?;
        let owned = match work {
            Some(work)
                if work.status == "running" && work.lease_token == Some(item.lease_token) =>
            {
                work
            }
            Some(work) if work.status != "running" => {
                return Ok(ProcessResult::Failed {
                    stage: "docling".to_string(),
                    message: "owning item left the running worker".to_string(),
                    retryable: false,
                });
            }
            _ => return Ok(lease_lost()),
        };
        let sections = match ops
            .remote_sections_with_view(file_id, converted, view)
            .await
        {
            Ok(sections) => sections,
            Err(error) => {
                if !error.retryable {
                    return Ok(ProcessResult::Failed {
                        stage: error.stage.clone(),
                        message: error.message.clone(),
                        retryable: false,
                    });
                }
                attempt += 1;
                if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                    return Ok(lease_lost());
                }
                continue;
            }
        };
        let payload = payload_with_sections(owned.payload, sections.clone());
        let execution = item.execution_context().ok_or_else(|| {
            anyhow::Error::from(DomainError::conflict("task item has no captured execution"))
        })?;
        match db
            .finish_docling_remote_job_with_sections(
                job.id,
                item.id,
                &execution,
                &payload,
                &status_name,
            )
            .await?
        {
            Some(_) => {
                // The same claim keeps driving this snapshot: publish the
                // committed sections into it so the `embedding` stage observes
                // the success instead of routing back to `docling` for a
                // duplicate submit. Externalization then removes only the
                // inline copy; on failure the durable inline checkpoint stays.
                item.payload = payload;
                ops.note_remote_success().await;
                ops.externalize_sections(item, &sections).await?;
                return Ok(ProcessResult::Progressed { next: "embedding" });
            }
            None => return Ok(lease_lost()),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{backoff_secs, is_expired};

    #[test]
    fn poll_backoff_is_bounded_and_starts_at_two_seconds() {
        for (attempt, expected) in [(0, 2), (1, 4), (2, 8), (3, 15), (4, 30), (100, 30)] {
            assert_eq!(backoff_secs(attempt), expected);
        }
    }

    #[test]
    fn past_deadline_is_expired_and_missing_is_not() {
        assert!(is_expired(Some(Utc::now() - chrono::Duration::seconds(1))));
        assert!(!is_expired(Some(Utc::now() + chrono::Duration::hours(1))));
        assert!(!is_expired(None));
    }
}
