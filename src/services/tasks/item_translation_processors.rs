use crate::domain_errors::DomainError;

use anyhow::{Context, Result};
use context69_contracts::{RebuildDocumentTranslationsRequest, TranslationStatus};

use super::TaskService;
use super::item_processors::{ProcessResult, process_error, set_stage};

pub(super) async fn process_translation(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    if stage == "finalize" {
        return Ok(ProcessResult::Succeeded(Some(task.id.to_string())));
    }
    let group = group.context(DomainError::invalid_argument(
        "translation tasks require group_id",
    ))?;
    if stage != "translation" {
        return Ok(process_error(
            stage,
            DomainError::invalid_argument(format!("unsupported translation task stage {stage}"))
                .into(),
        ));
    }
    let request: TranslationTaskItem = match serde_json::from_value(item.payload.clone()) {
        Ok(request) => request,
        Err(error) => return Ok(process_error(stage, error.into())),
    };
    // `rebuild_document` is a blocking conversion: it reuses or inserts the
    // document's jobs and runs them to completion before returning, so the
    // terminal status is already known here.
    let jobs = match service
        .translation()
        .rebuild_document(
            group.id,
            request.document_id,
            &RebuildDocumentTranslationsRequest {
                target_locales: request.target_locales,
            },
        )
        .await
    {
        Ok(response) => response.jobs,
        Err(error) => return Ok(process_error(stage, error)),
    };
    for job in jobs {
        if matches!(
            job.status,
            TranslationStatus::Failed
                | TranslationStatus::QuotaExceeded
                | TranslationStatus::Unavailable
        ) {
            return Ok(ProcessResult::Failed {
                stage: stage.to_string(),
                message: job
                    .error_message
                    .unwrap_or_else(|| "translation failed".to_string()),
                retryable: matches!(
                    job.status,
                    TranslationStatus::Unavailable | TranslationStatus::QuotaExceeded
                ),
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
}
