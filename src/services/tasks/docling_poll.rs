use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::db::{Database, StoredDoclingRemoteJob};
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
    async fn poll_remote(
        &self,
        remote_task_id: &str,
    ) -> Result<docling_convert::TaskStatusResponse, UnifiedIngestError>;
    async fn fetch_remote(
        &self,
        file_id: Uuid,
        remote_task_id: &str,
    ) -> Result<docling_convert::ConvertedDocument, UnifiedIngestError>;
    async fn remote_sections(
        &self,
        file_id: Uuid,
        converted: docling_convert::ConvertedDocument,
    ) -> Result<Value, UnifiedIngestError>;
    async fn note_remote_failure(&self, error: &UnifiedIngestError);
    async fn note_remote_success(&self);
}

#[async_trait::async_trait]
impl DoclingRemoteOps for LibraryService {
    async fn poll_remote(
        &self,
        remote_task_id: &str,
    ) -> Result<docling_convert::TaskStatusResponse, UnifiedIngestError> {
        self.poll_docling_remote(remote_task_id).await
    }

    async fn fetch_remote(
        &self,
        file_id: Uuid,
        remote_task_id: &str,
    ) -> Result<docling_convert::ConvertedDocument, UnifiedIngestError> {
        self.fetch_docling_remote(file_id, remote_task_id).await
    }

    async fn remote_sections(
        &self,
        file_id: Uuid,
        converted: docling_convert::ConvertedDocument,
    ) -> Result<Value, UnifiedIngestError> {
        self.sections_for_remote_result(file_id, converted).await
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
/// item left `running` (cancelled, failed, or requeued elsewhere) and the
/// remote reference must be fenced; `Stolen` means another worker holds the
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
/// - lost ownership (cancel, reclaim): fence the remote row when the item is
///   gone, otherwise write nothing; the fenced item finish observes the loss.
pub(super) async fn run_docling_stage_blocking(
    db: &Database,
    ops: &(impl DoclingRemoteOps + '_),
    item: &mut crate::db::ClaimedItem,
    file_id: Uuid,
    job: StoredDoclingRemoteJob,
) -> Result<ProcessResult> {
    let mut attempt: i32 = 0;
    loop {
        match owner_state(db, item.id, item.lease_token).await? {
            OwnerState::Gone => {
                db.cancel_active_docling_remote_job_for_item(
                    item.id,
                    Some("owning item left the running worker"),
                )
                .await?;
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
        let status = match ops.poll_remote(&job.remote_task_id).await {
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
        let status_name = format!("{:?}", status.task_status).to_ascii_lowercase();
        if !status.task_status.is_terminal() {
            attempt += 1;
            if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                return Ok(lease_lost());
            }
            continue;
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
        let converted = match ops.fetch_remote(file_id, &job.remote_task_id).await {
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
                db.cancel_active_docling_remote_job_for_item(
                    item.id,
                    Some("owning item left the running worker"),
                )
                .await?;
                return Ok(ProcessResult::Failed {
                    stage: "docling".to_string(),
                    message: "owning item left the running worker".to_string(),
                    retryable: false,
                });
            }
            _ => return Ok(lease_lost()),
        };
        let sections = match ops.remote_sections(file_id, converted).await {
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
        let payload = payload_with_sections(owned.payload, sections);
        match db
            .finish_docling_remote_job_with_sections(
                job.id,
                item.id,
                item.lease_token,
                &payload,
                &status_name,
            )
            .await?
        {
            Some(_) => {
                // The same claim keeps driving this snapshot: publish the
                // committed sections into it so the `embedding` stage observes
                // the success instead of routing back to `docling` for a
                // duplicate submit.
                item.payload = payload;
                ops.note_remote_success().await;
                return Ok(ProcessResult::Progressed { next: "embedding" });
            }
            None => return Ok(lease_lost()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use chrono::Utc;
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::{DoclingRemoteOps, backoff_secs, is_expired, run_docling_stage_blocking};
    use crate::services::library::UnifiedIngestError;
    use crate::services::tasks::item_processors::{ProcessResult, persisted_section_payload};

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

    /// Scripted remote double: polling/fetching/section answers are canned so
    /// the real blocking loop runs without a live converter.
    struct ScriptedRemote {
        polls: Mutex<VecDeque<Result<docling_convert::TaskStatusResponse, UnifiedIngestError>>>,
        fetch: Mutex<Option<Result<docling_convert::ConvertedDocument, UnifiedIngestError>>>,
        sections: Mutex<Option<Result<Value, UnifiedIngestError>>>,
    }

    impl ScriptedRemote {
        fn pending_then_success() -> Self {
            Self {
                polls: Mutex::new(
                    [
                        Ok(status(docling_convert::ConversionStatus::Started)),
                        Ok(status(docling_convert::ConversionStatus::Success)),
                    ]
                    .into(),
                ),
                fetch: Mutex::new(Some(Ok(docling_convert::ConvertedDocument {
                    filename: "a.pdf".to_string(),
                    markdown: Some("# hi".to_string()),
                    text: None,
                    json: None,
                    html: None,
                    doctags: None,
                    doclang: None,
                    chunks: Vec::new(),
                    chunk_response: None,
                    archive: None,
                    metadata: docling_convert::ConvertedDocumentMetadata {
                        input_kind: docling_convert::InputKind::Pdf,
                        media_type: "application/pdf".to_string(),
                    },
                    errors: Vec::new(),
                }))),
                sections: Mutex::new(Some(Ok(json!([{"section_key": "document"}])))),
            }
        }
    }

    fn status(
        task_status: docling_convert::ConversionStatus,
    ) -> docling_convert::TaskStatusResponse {
        docling_convert::TaskStatusResponse {
            task_id: "remote-1".to_string(),
            task_type: None,
            task_status,
            task_position: None,
            task_meta: None,
            error_message: None,
            failure: None,
        }
    }

    #[async_trait::async_trait]
    impl DoclingRemoteOps for ScriptedRemote {
        async fn poll_remote(
            &self,
            _remote_task_id: &str,
        ) -> Result<docling_convert::TaskStatusResponse, UnifiedIngestError> {
            self.polls
                .lock()
                .expect("poll script")
                .pop_front()
                .expect("poll script exhausted")
        }

        async fn fetch_remote(
            &self,
            _file_id: Uuid,
            _remote_task_id: &str,
        ) -> Result<docling_convert::ConvertedDocument, UnifiedIngestError> {
            self.fetch
                .lock()
                .expect("fetch script")
                .take()
                .expect("fetch used twice")
        }

        async fn remote_sections(
            &self,
            _file_id: Uuid,
            _converted: docling_convert::ConvertedDocument,
        ) -> Result<Value, UnifiedIngestError> {
            self.sections
                .lock()
                .expect("sections script")
                .take()
                .expect("sections used twice")
        }

        async fn note_remote_failure(&self, _error: &UnifiedIngestError) {}

        async fn note_remote_success(&self) {}
    }

    /// Stage-level regression for the P0: the real blocking loop commits a
    /// terminal success and the SAME snapshot carries the sections, so the
    /// `embedding` pre-advance guard observes them and cannot route back to
    /// `docling` for a duplicate submit.
    #[tokio::test]
    async fn blocking_success_updates_the_worker_snapshot() {
        let Some(db) = test_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
            return;
        };
        let (user_id, task_id, mut item, file_id) = seed_running_item(&db).await;
        let job = db
            .create_docling_remote_job(
                task_id,
                item.id,
                &format!("docling-{}", Uuid::new_v4()),
                Some("submitted"),
                Some(Utc::now() - chrono::Duration::seconds(1)),
                Some(Utc::now() + chrono::Duration::hours(1)),
            )
            .await
            .expect("remote job");
        let ops = ScriptedRemote::pending_then_success();
        let outcome = run_docling_stage_blocking(&db, &ops, &mut item, file_id, job)
            .await
            .expect("blocking run");
        assert!(
            matches!(outcome, ProcessResult::Progressed { next } if next == "embedding"),
            "terminal success must advance to embedding"
        );
        let sections = persisted_section_payload(&item.payload)
            .expect("the worker snapshot must carry the committed sections");
        assert_eq!(sections, json!([{"section_key": "document"}]));
        assert!(
            db.get_active_docling_remote_job_for_item(item.id)
                .await
                .expect("active")
                .is_none(),
            "committed row leaves no active reference to resubmit"
        );
        db.cancel_active_docling_remote_job_for_item(item.id, Some("test done"))
            .await
            .expect("cancel remote");
        cleanup(&db, task_id, user_id).await;
    }

    async fn test_db() -> Option<crate::db::Database> {
        let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
        Some(
            crate::db::Database::connect(&url)
                .await
                .expect("connect test database"),
        )
    }

    /// Seeds a running item under a lease and returns the snapshot the worker
    /// would drive, plus a placeholder file id (the scripted remote never
    /// touches storage).
    async fn seed_running_item(
        db: &crate::db::Database,
    ) -> (i64, uuid::Uuid, crate::db::ClaimedItem, uuid::Uuid) {
        use serde_json::json;
        use sqlx::Row;

        use crate::db::CreateTaskSubmissionRequest;

        let user_id: i64 = sqlx::query(
            "INSERT INTO context69.users (login_name, display_name, password_hash) \
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(format!("docling-stage-{}", uuid::Uuid::new_v4()))
        .bind("Docling Stage Test")
        .bind("unused")
        .fetch_one(db.pool())
        .await
        .expect("seed user")
        .get("id");
        let task_id = uuid::Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: None,
                kind: "text_batch",
                group_path: Some("test/docling-stage"),
                source_key: None,
                payloads: &[json!({"external_id": "a"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("docling-stage-{}", uuid::Uuid::new_v4()),
            })
            .await
            .expect("create task");
        let item_id = items[0];
        let lease = uuid::Uuid::new_v4();
        let file_id = uuid::Uuid::new_v4();
        sqlx::query(
            "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
             lease_until = now() + interval '5 minutes', attempt_count = 1 WHERE id = $1",
        )
        .bind(item_id)
        .bind(lease)
        .execute(db.pool())
        .await
        .expect("claim-like setup");
        let item = crate::db::ClaimedItem {
            id: item_id,
            task_id,
            ordinal: 0,
            attempt_count: 1,
            lease_token: lease,
            attempt_id: 1,
            payload: json!({"external_id": "a"}),
            file_id: None,
            stage: Some("processing".to_string()),
            input_storage_object_id: None,
            kind: "text_batch".to_string(),
            group_id: None,
            group_path: Some("test/docling-stage".to_string()),
            source_key: None,
        };
        (user_id, task_id, item, file_id)
    }

    async fn cleanup(db: &crate::db::Database, task_id: Uuid, user_id: i64) {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("cleanup items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("cleanup task");
        sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup idempotency");
        sqlx::query("DELETE FROM context69.users WHERE id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup user");
    }
}
