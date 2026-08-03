use anyhow::{Context, Result, anyhow};
use chrono::{Duration as ChronoDuration, Utc};
use context69_contracts::{RebuildDocumentTranslationsRequest, TranslationStatus};
use serde_json::json;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{ProcessResult, process_error, set_stage};

pub(super) async fn process_translation(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedTaskItem,
    stage: &str,
) -> Result<ProcessResult> {
    if stage == "finalize" {
        return Ok(ProcessResult::Succeeded(Some(task.id.to_string())));
    }
    let group = group.context("translation tasks require group_id")?;
    if stage != "translation" {
        return Ok(process_error(
            stage,
            anyhow!("unsupported translation task stage {stage}"),
        ));
    }
    let request: TranslationTaskItem = match serde_json::from_value(item.payload.clone()) {
        Ok(request) => request,
        Err(error) => return Ok(process_error(stage, error.into())),
    };
    let job_ids = match request.job_ids.clone() {
        Some(job_ids) => job_ids,
        None => {
            let jobs = service
                .translation()
                .rebuild_document(
                    group.id,
                    request.document_id,
                    &RebuildDocumentTranslationsRequest {
                        target_locales: request.target_locales,
                    },
                )
                .await
                .map_err(|error| anyhow!(error.to_string()))?
                .jobs;
            let job_ids = jobs.iter().map(|job| job.job_id).collect::<Vec<_>>();
            let mut payload = item.payload.clone();
            payload["job_ids"] = json!(job_ids);
            if !service
                .db()
                .set_task_item_payload(item.id, item.lease_token, &payload)
                .await?
            {
                return Err(anyhow!(
                    "task item lease was lost while saving translation jobs"
                ));
            }
            job_ids
        }
    };
    for job_id in job_ids {
        let current = service.translation().job(group.id, job_id).await?;
        if matches!(
            current.status,
            TranslationStatus::Failed
                | TranslationStatus::QuotaExceeded
                | TranslationStatus::Unavailable
        ) {
            return Ok(ProcessResult::Failed {
                stage: stage.to_string(),
                message: current
                    .error_message
                    .unwrap_or_else(|| "translation failed".to_string()),
                retryable: matches!(
                    current.status,
                    TranslationStatus::Unavailable | TranslationStatus::QuotaExceeded
                ),
            });
        }
        if matches!(
            current.status,
            TranslationStatus::Queued | TranslationStatus::Running
        ) {
            return Ok(ProcessResult::Waiting {
                reason: "external_job".to_string(),
                dependency_key: Some("translation".to_string()),
                next_attempt_at: Utc::now() + ChronoDuration::seconds(5),
                message: Some("translation job is still running".to_string()),
            });
        }
    }
    set_stage(service, task, item, "finalize").await?;
    Ok(ProcessResult::Progressed)
}

#[derive(Debug, serde::Deserialize)]
struct TranslationTaskItem {
    document_id: i64,
    #[serde(default)]
    target_locales: Vec<String>,
    #[serde(default)]
    job_ids: Option<Vec<Uuid>>,
}
