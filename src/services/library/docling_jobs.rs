use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use docling_convert::{ConversionStatus, InputDocument, PdfConvertError};
use serde_json::Value;
use tokio::time::timeout;
use uuid::Uuid;

use super::dependency_runtime::{
    docling_error_status_code, is_docling_remote_task_not_found, is_docling_transient,
};
use super::ingest_documents::sections_from_converted_document;
use super::task_ingest::{normalize_task_failure, task_failure};
use super::*;

pub(crate) const DOCLING_EXTERNAL_JOB_PROVIDER: &str = "docling";

/// Minimum spacing between Docling long-poll checks. Docling blocks each poll
/// for up to 30 seconds server-side, so a shorter cadence would only churn
/// worker slots without gaining freshness.
const MIN_DOCLING_POLL_CADENCE: Duration = Duration::from_secs(30);

pub(crate) struct DoclingJobSubmitted {
    pub external_job_id: Uuid,
    pub remote_task_id: String,
    pub next_poll_at: DateTime<Utc>,
    pub submission_count: i32,
}

pub(crate) enum DoclingPollOutcome {
    Pending {
        next_poll_at: DateTime<Utc>,
    },
    Success {
        sections: Value,
    },
    /// Terminal poll outcome that failed this item. `retryable=true` means
    /// the underlying cause was a transient Docling problem and the worker
    /// should re-enter the dependency wait instead of failing the item.
    Failed {
        message: String,
        retryable: bool,
        dependency_key: Option<String>,
    },
    /// The persisted job is missing or in a terminal state (timed out, failed,
    /// cancelled); the item must submit a fresh job. Only reached on manual
    /// retry, task recovery, or after an HTTP 404 from a stale remote id.
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

        if let Some(job) = self
            .store
            .get_external_job(item_id, DOCLING_EXTERNAL_JOB_PROVIDER)
            .await
            .map_err(|error| task_failure("docling", error, true))?
        {
            let now = Utc::now();
            if job.is_active() && job.deadline_at.is_none_or(|deadline| deadline > now) {
                self.note_dependency_success(LibraryDependency::Docling, lease_token)
                    .await;
                return Ok(DoclingJobSubmitted {
                    external_job_id: job.id,
                    remote_task_id: job.remote_task_id,
                    next_poll_at: job.next_poll_at,
                    submission_count: job.submission_count,
                });
            }
            if job.is_active() {
                self.store
                    .supersede_external_job(
                        item_id,
                        DOCLING_EXTERNAL_JOB_PROVIDER,
                        "Docling external job deadline expired; submitting a fresh job",
                    )
                    .await
                    .map_err(|error| task_failure("docling", error, true))?;
            }
            if job.is_submitting() {
                return Err(task_failure(
                    "docling",
                    anyhow!(
                        "Docling submission outcome is uncertain for item {item_id}; manual recovery is required"
                    ),
                    false,
                ));
            }
        }

        let bytes = self
            .read_active_storage_for_lease(&file.storage_rel_path, lease_token)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .with_context(|| format!("stored file not found for file {file_id}"))
            .map_err(|error| task_failure("storage", error, false))?;

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
        let permit = self
            .acquire_docling_permit()
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        let submission_marker = format!("submitting-{}", Uuid::new_v4());
        let submission = self
            .store
            .begin_external_job_submission(
                item_id,
                DOCLING_EXTERNAL_JOB_PROVIDER,
                &submission_marker,
                next_poll_at,
                deadline,
            )
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes);
        let handle = match converter.submit_async(input).await {
            Ok(handle) => handle,
            Err(error) => {
                return Err(task_failure(
                    "docling",
                    anyhow!(
                        "Docling submission outcome is uncertain for item {item_id}: {error}; manual recovery is required"
                    ),
                    false,
                ));
            }
        };
        drop(permit);

        let submission = self
            .store
            .complete_external_job_submission(submission.id, handle.task_id(), next_poll_at)
            .await
            // Docling has accepted the remote job at this point. If recording
            // its id fails, retrying blindly could create a second conversion.
            .map_err(|error| task_failure("docling", error, false))?;
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
            external_job_id: submission.id,
            remote_task_id: handle.task_id().to_string(),
            next_poll_at,
            submission_count: submission.submission_count,
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
        if job.is_submitting() {
            return Ok(DoclingPollOutcome::Failed {
                message: format!(
                    "Docling submission outcome is uncertain for item {item_id}; manual recovery is required"
                ),
                retryable: false,
                dependency_key: None,
            });
        }
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
        if job
            .deadline_at
            .is_some_and(|deadline| Utc::now() >= deadline)
        {
            return mark_docling_job_timed_out(
                self,
                job.id,
                &job.remote_task_id,
                job.remote_status.as_deref(),
            )
            .await;
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
        let status = match timeout(
            config.connection.timeout.min(Duration::from_secs(
                crate::docling::DEFAULT_DOCLING_TIMEOUT_SECS,
            )),
            converter.poll_remote(&job.remote_task_id),
        )
        .await
        {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                let context = anyhow!(
                    "failed to poll docling task {}: {error}",
                    job.remote_task_id
                );
                if job
                    .deadline_at
                    .is_some_and(|deadline| Utc::now() >= deadline)
                {
                    return mark_docling_job_timed_out(
                        self,
                        job.id,
                        &job.remote_task_id,
                        job.remote_status.as_deref(),
                    )
                    .await;
                }
                return Ok(classify_docling_poll_error(
                    job.id,
                    &job.remote_task_id,
                    Some(&error),
                    context,
                    lease_token,
                    self,
                )
                .await);
            }
            Err(_) => {
                let context = anyhow!(
                    "polling Docling task {} exceeded the {} second request timeout",
                    job.remote_task_id,
                    config
                        .connection
                        .timeout
                        .min(Duration::from_secs(
                            crate::docling::DEFAULT_DOCLING_TIMEOUT_SECS,
                        ))
                        .as_secs()
                );
                if job
                    .deadline_at
                    .is_some_and(|deadline| Utc::now() >= deadline)
                {
                    return mark_docling_job_timed_out(
                        self,
                        job.id,
                        &job.remote_task_id,
                        job.remote_status.as_deref(),
                    )
                    .await;
                }
                return Ok(classify_docling_poll_error(
                    job.id,
                    &job.remote_task_id,
                    None,
                    context,
                    lease_token,
                    self,
                )
                .await);
            }
        };
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
                    return mark_docling_job_timed_out_with_message(
                        self,
                        job.id,
                        &message,
                        Some(conversion_status_str(status.task_status)),
                    )
                    .await;
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
                Ok(DoclingPollOutcome::Failed {
                    message,
                    retryable: false,
                    dependency_key: None,
                })
            }
            ConversionStatus::Success | ConversionStatus::PartialSuccess => {
                let file = self.task_file_for_job(file_id).await?;
                let input = InputDocument::new(&file.filename, &file.media_type, Bytes::new());
                let document = match converter.fetch_remote(input, &job.remote_task_id).await {
                    Ok(document) => document,
                    Err(error) => {
                        let context = anyhow!(
                            "failed to fetch result of docling task {}: {error}",
                            job.remote_task_id
                        );
                        return Ok(classify_docling_poll_error(
                            job.id,
                            &job.remote_task_id,
                            Some(&error),
                            context,
                            lease_token,
                            self,
                        )
                        .await);
                    }
                };
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

async fn mark_docling_job_timed_out(
    service: &LibraryService,
    external_job_id: Uuid,
    remote_task_id: &str,
    remote_status: Option<&str>,
) -> Result<DoclingPollOutcome, UnifiedIngestError> {
    let message = format!(
        "docling task {remote_task_id} did not finish before its deadline; resubmit the item manually"
    );
    mark_docling_job_timed_out_with_message(service, external_job_id, &message, remote_status).await
}

async fn mark_docling_job_timed_out_with_message(
    service: &LibraryService,
    external_job_id: Uuid,
    message: &str,
    remote_status: Option<&str>,
) -> Result<DoclingPollOutcome, UnifiedIngestError> {
    service
        .store
        .update_external_job(
            external_job_id,
            "timed_out",
            remote_status,
            Utc::now(),
            Some(message),
        )
        .await
        .map_err(|error| task_failure("docling_poll", error, true))?;
    Ok(DoclingPollOutcome::Failed {
        message: message.to_string(),
        retryable: false,
        dependency_key: None,
    })
}

/// Map a Docling poll error to the correct outcome.
///
/// - HTTP 404 (the remote task id is gone): ask the worker to resubmit
///   instead of continuing to poll a dead id; this is the failure mode that
///   stranded the two canary tasks in `docling_poll` indefinitely.
/// - Transient transport failures (timeout, connection refused, 5xx, 429):
///   open the Docling dependency gate and emit a retryable `Failed` so the
///   runtime translates it into a short dependency-wait rather than an
///   item-level failed status.
/// - Configuration / auth failures (401, 403, not configured): fail the item
///   permanently; resubmitting would only hit the same wall.
async fn classify_docling_poll_error(
    external_job_id: Uuid,
    remote_task_id: &str,
    docling_error: Option<&PdfConvertError>,
    context_error: anyhow::Error,
    lease_token: Uuid,
    service: &LibraryService,
) -> DoclingPollOutcome {
    if docling_error.is_some_and(is_docling_remote_task_not_found) {
        return DoclingPollOutcome::ResubmitRequired {
            message: format!(
                "docling task {remote_task_id} no longer exists (HTTP 404); resubmitting a fresh job"
            ),
        };
    }
    if let Some(status) = docling_error.and_then(docling_error_status_code)
        && matches!(status, 401 | 403)
    {
        let message = format!(
            "docling rejected the poll request with HTTP {status}; check the Docling configuration and retry"
        );
        let status_text = status.to_string();
        if let Err(error) = service
            .store
            .update_external_job(
                external_job_id,
                "failure",
                Some(&status_text),
                Utc::now(),
                Some(&message),
            )
            .await
        {
            tracing::warn!(%error, "failed to persist permanent Docling poll error");
        }
        return DoclingPollOutcome::Failed {
            message,
            retryable: false,
            dependency_key: None,
        };
    }

    let transient = is_docling_transient(docling_error, &context_error);
    let message = format!("{context_error}");
    if let Err(error) = service
        .store
        .update_external_job(
            external_job_id,
            if transient { "running" } else { "failure" },
            docling_error
                .and_then(docling_error_status_code)
                .map(|code| code.to_string())
                .as_deref(),
            Utc::now() + chrono::Duration::seconds(60),
            Some(&message),
        )
        .await
    {
        tracing::warn!(
            %error,
            "failed to persist docling poll error on external job"
        );
    }
    if transient {
        service
            .note_dependency_failure_with_lease(
                LibraryDependency::Docling,
                lease_token,
                &context_error,
            )
            .await;
        DoclingPollOutcome::Failed {
            message: format!(
                "docling poll failed transiently; waiting for dependency gate (status=open): {message}"
            ),
            retryable: true,
            dependency_key: Some(LibraryDependency::Docling.as_str().to_string()),
        }
    } else {
        DoclingPollOutcome::Failed {
            message: format!("docling poll failed: {message}"),
            retryable: false,
            dependency_key: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use docling_convert::PdfConvertError;

    use crate::services::library::dependency_runtime::{
        docling_error_status_code, is_docling_remote_task_not_found,
        is_docling_transient_error_for_test,
    };

    fn api_error(status: u16, message: &str) -> PdfConvertError {
        PdfConvertError::api_error(Some(status), message)
    }

    #[test]
    fn http_404_is_never_treated_as_transient() {
        let error = api_error(404, "task not found");
        assert!(!is_docling_transient_error_for_test(
            Some(&error),
            &anyhow!("poll")
        ));
        assert_eq!(docling_error_status_code(&error), Some(404));
        assert!(is_docling_remote_task_not_found(&error));
    }

    #[test]
    fn http_5xx_and_429_are_treated_as_transient() {
        for status in [500u16, 502, 503, 504, 429] {
            let error = api_error(status, "upstream");
            assert!(
                is_docling_transient_error_for_test(Some(&error), &anyhow!("poll")),
                "expected transient classification for status {status}"
            );
            assert!(!is_docling_remote_task_not_found(&error));
        }
    }

    #[test]
    fn http_401_and_403_are_never_treated_as_transient() {
        for status in [401u16, 403] {
            let error = api_error(status, "unauthorized");
            assert!(!is_docling_transient_error_for_test(
                Some(&error),
                &anyhow!("poll")
            ));
        }
    }

    #[test]
    fn transient_string_messages_still_match_when_status_is_unknown() {
        for message in [
            "connection refused",
            "request timed out",
            "service unavailable (status 503)",
            "http 429 rate limit",
        ] {
            let error = anyhow!("docling poll: {message}");
            assert!(
                is_docling_transient_error_for_test(None, &error),
                "expected transient classification for {message}"
            );
        }
    }
}
