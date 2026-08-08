use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use docling_convert::{ConversionStatus, InputDocument};
use serde_json::Value;
use uuid::Uuid;

use super::ingest_documents::sections_from_converted_document;
use super::task_ingest::{normalize_task_failure, task_failure, task_failure_with_dependency};
use super::*;

pub(crate) const DOCLING_EXTERNAL_JOB_PROVIDER: &str = "docling";

/// Minimum spacing between Docling long-poll checks. Docling blocks each poll
/// for up to 30 seconds server-side, so a shorter cadence would only churn
/// worker slots without gaining freshness.
const MIN_DOCLING_POLL_CADENCE: Duration = Duration::from_secs(30);

pub(crate) struct DoclingJobSubmitted {
    pub remote_task_id: String,
    pub next_poll_at: DateTime<Utc>,
}

pub(crate) enum DoclingPollOutcome {
    Pending {
        next_poll_at: DateTime<Utc>,
    },
    Success {
        sections: Value,
    },
    Failed {
        message: String,
    },
    /// The persisted job is missing or in a terminal state (timed out, failed,
    /// cancelled); the item must submit a fresh job. Only reached on manual
    /// retry or task recovery, never as an automatic loop.
    ResubmitRequired {
        message: String,
    },
}

impl LibraryService {
    /// Submit a whole-document Docling conversion for `file_id` and persist the
    /// remote task id. The item then parks on `external_job` polling instead of
    /// occupying a worker for the whole conversion.
    pub(crate) async fn submit_docling_job_for_task(
        &self,
        item_id: Uuid,
        file_id: Uuid,
        lease_token: Uuid,
        task_id: Uuid,
    ) -> Result<DoclingJobSubmitted, UnifiedIngestError> {
        let file = self.task_file_for_job(file_id).await?;
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| task_failure("parsing", error, false))?;
        if kind == LibraryFileKind::PlainText {
            return Err(task_failure(
                "docling",
                anyhow!("plain text cannot be submitted to docling"),
                false,
            ));
        }

        let bytes = self
            .read_active_storage_for_lease(&file.storage_rel_path, lease_token)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .with_context(|| format!("stored file not found for file {file_id}"))
            .map_err(|error| task_failure("storage", error, false))?;

        let permit = self
            .acquire_docling_permit()
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes);
        let handle = converter
            .submit_async(input)
            .await
            .map_err(|error| task_failure_with_dependency("docling", anyhow!(error), "docling"))?;
        drop(permit);

        let config = self
            .settings
            .resolve_docling_config()
            .await
            .map_err(|error| task_failure("docling", error, true))?
            .context("docling is not configured")
            .map_err(|error| task_failure("docling", error, false))?;
        let now = Utc::now();
        let deadline = now
            + chrono::Duration::from_std(config.connection.task_timeout)
                .unwrap_or_else(|_| chrono::Duration::seconds(3600));
        let next_poll_at = now + poll_cadence(config.connection.poll_interval);
        self.store
            .upsert_external_job(
                item_id,
                DOCLING_EXTERNAL_JOB_PROVIDER,
                handle.task_id(),
                "pending",
                next_poll_at,
                deadline,
            )
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        self.note_dependency_success(LibraryDependency::Docling, lease_token)
            .await;
        info!(
            target: "docling",
            task_id = %task_id,
            item_id = %item_id,
            file_name = %file.filename,
            remote_task_id = %handle.task_id(),
            deadline = %deadline,
            "docling whole-document job submitted"
        );
        Ok(DoclingJobSubmitted {
            remote_task_id: handle.task_id().to_string(),
            next_poll_at,
        })
    }

    /// Poll a persisted whole-document Docling job. Returns either a new
    /// polling time, the parsed sections on success, or a terminal failure.
    /// Connection-level errors are routed to the docling dependency gate; a
    /// missed deadline only fails this item and never touches the gate.
    pub(crate) async fn poll_docling_job_for_task(
        &self,
        item_id: Uuid,
        file_id: Uuid,
        lease_token: Uuid,
    ) -> Result<DoclingPollOutcome, UnifiedIngestError> {
        let job = match self
            .store
            .get_external_job(item_id, DOCLING_EXTERNAL_JOB_PROVIDER)
            .await
            .map_err(|error| task_failure("docling_poll", error, true))?
        {
            Some(job) => job,
            None => {
                return Ok(DoclingPollOutcome::ResubmitRequired {
                    message: format!(
                        "docling external job is missing for item {item_id}; resubmitting"
                    ),
                });
            }
        };
        if !job.is_active() {
            return Ok(DoclingPollOutcome::ResubmitRequired {
                message: match job.error_message {
                    Some(error) => format!(
                        "docling job {} is in state {} ({error}); resubmitting the item",
                        job.remote_task_id, job.status
                    ),
                    None => format!(
                        "docling job {} is in state {}; resubmitting the item",
                        job.remote_task_id, job.status
                    ),
                },
            });
        }
        if Utc::now() < job.next_poll_at {
            // A concurrent worker may have claimed this item early; respect the
            // stored cadence instead of hitting Docling ahead of schedule.
            return Ok(DoclingPollOutcome::Pending {
                next_poll_at: job.next_poll_at,
            });
        }

        let config = self
            .settings
            .resolve_docling_config()
            .await
            .map_err(|error| task_failure("docling", error, true))?
            .context("docling is not configured")
            .map_err(|error| task_failure("docling", error, false))?;
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        let status = converter
            .poll_remote(&job.remote_task_id)
            .await
            .map_err(|error| {
                task_failure_with_dependency(
                    "docling_poll",
                    anyhow!(
                        "failed to poll docling task {}: {error}",
                        job.remote_task_id
                    ),
                    "docling",
                )
            })?;
        let now = Utc::now();
        let next_poll_at = now + poll_cadence(config.connection.poll_interval);
        match status.task_status {
            ConversionStatus::Pending | ConversionStatus::Started => {
                if job.deadline_at.is_some_and(|deadline| now >= deadline) {
                    let message = match job.remote_status {
                        Some(remote_status) => format!(
                            "docling task {} did not finish before its deadline (still {}); resubmit the item manually",
                            job.remote_task_id, remote_status
                        ),
                        None => format!(
                            "docling task {} did not finish before its deadline; resubmit the item manually",
                            job.remote_task_id
                        ),
                    };
                    self.store
                        .update_external_job(
                            job.id,
                            "timed_out",
                            Some(conversion_status_str(status.task_status)),
                            now + chrono::Duration::seconds(3600),
                            Some(&message),
                        )
                        .await
                        .map_err(|error| task_failure("docling_poll", error, true))?;
                    return Ok(DoclingPollOutcome::Failed { message });
                }
                self.store
                    .update_external_job(
                        job.id,
                        "running",
                        Some(conversion_status_str(status.task_status)),
                        next_poll_at,
                        None,
                    )
                    .await
                    .map_err(|error| task_failure("docling_poll", error, true))?;
                Ok(DoclingPollOutcome::Pending { next_poll_at })
            }
            ConversionStatus::Failure | ConversionStatus::Skipped => {
                let message = status
                    .error_message
                    .clone()
                    .or_else(|| {
                        status
                            .failure
                            .as_ref()
                            .map(|failure| failure.message.clone())
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "docling task {} failed with status {}",
                            job.remote_task_id,
                            conversion_status_str(status.task_status)
                        )
                    });
                self.store
                    .update_external_job(
                        job.id,
                        "failure",
                        Some(conversion_status_str(status.task_status)),
                        next_poll_at,
                        Some(&message),
                    )
                    .await
                    .map_err(|error| task_failure("docling_poll", error, true))?;
                Ok(DoclingPollOutcome::Failed { message })
            }
            ConversionStatus::Success | ConversionStatus::PartialSuccess => {
                let file = self.task_file_for_job(file_id).await?;
                let input = InputDocument::new(&file.filename, &file.media_type, Bytes::new());
                let document = converter
                    .fetch_remote(input, &job.remote_task_id)
                    .await
                    .map_err(|error| {
                        task_failure(
                            "docling_poll",
                            anyhow!(
                                "failed to fetch result of docling task {}: {error}",
                                job.remote_task_id
                            ),
                            true,
                        )
                    })?;
                let sections = sections_from_converted_document(&file, document)
                    .map_err(normalize_task_failure)?;
                let sections = serde_json::to_value(sections)
                    .map_err(|error| task_failure("docling_poll", error, false))?;
                self.store
                    .update_external_job(
                        job.id,
                        "success",
                        Some(conversion_status_str(status.task_status)),
                        next_poll_at,
                        None,
                    )
                    .await
                    .map_err(|error| task_failure("docling_poll", error, true))?;
                self.note_dependency_success(LibraryDependency::Docling, lease_token)
                    .await;
                Ok(DoclingPollOutcome::Success { sections })
            }
        }
    }

    async fn task_file_for_job(
        &self,
        file_id: Uuid,
    ) -> Result<crate::domain::LibraryFileRecord, UnifiedIngestError> {
        self.store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .with_context(|| format!("unknown file {file_id}"))
            .map_err(|error| task_failure("storage", error, false))
    }
}

fn poll_cadence(poll_interval: Duration) -> chrono::Duration {
    let cadence = poll_interval.max(MIN_DOCLING_POLL_CADENCE);
    chrono::Duration::from_std(cadence).unwrap_or_else(|_| chrono::Duration::seconds(30))
}

fn conversion_status_str(status: ConversionStatus) -> &'static str {
    match status {
        ConversionStatus::Pending => "pending",
        ConversionStatus::Started => "started",
        ConversionStatus::Failure => "failure",
        ConversionStatus::Success => "success",
        ConversionStatus::PartialSuccess => "partial_success",
        ConversionStatus::Skipped => "skipped",
    }
}
