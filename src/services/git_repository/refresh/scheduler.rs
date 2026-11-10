//! Scheduler tick and refresh admission.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use uuid::Uuid;

use crate::contracts::{TaskKind, TaskRef};
use crate::domain_errors::DomainError;
use crate::services::app::Context69App;
use crate::services::tasks::TaskSubmission;

use super::cleanup::{retry_pending_deletions, retry_pending_generation_cleanup};
use super::{GIT_REFRESH_BATCH, GIT_REFRESH_TICK};

/// Starts the always-on Git refresh scheduler.
///
/// It is independent of the document/vector runtime gate: a source's interval
/// and webhook demand are Git-specific, and each task re-evaluates its target
/// when it runs.
pub(crate) async fn run_git_refresh_scheduler(app: Arc<Context69App>) {
    let mut ticker = tokio::time::interval(GIT_REFRESH_TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(error) = refresh_due_sources(app.as_ref()).await {
            tracing::warn!(%error, "git refresh scheduler pass failed");
        }
        if let Err(error) = retry_pending_deletions(app.as_ref()).await {
            tracing::warn!(%error, "git repository deletion retry pass failed");
        }
        if let Err(error) = retry_pending_generation_cleanup(app.as_ref()).await {
            tracing::warn!(%error, "git generation cleanup retry pass failed");
        }
    }
}

/// Admits every due source once, bounded per pass.
pub(crate) async fn refresh_due_sources(app: &Context69App) -> Result<()> {
    let due = app
        .db
        .list_due_git_refresh_sources(GIT_REFRESH_BATCH)
        .await?;
    for (group_id, repository_key) in due {
        match enqueue_refresh(app, group_id, repository_key).await {
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    %repository_key,
                    %error,
                    "due git source could not be admitted"
                );
            }
        }
    }
    Ok(())
}

/// Records a demand and admits one repository refresh.
///
/// The demand is written before admission so even an already running task
/// leaves a follow-up. A failed admission keeps the demand for the scheduler.
pub(crate) async fn request_and_enqueue_refresh(
    app: &Context69App,
    group_id: i64,
    repository_key: Uuid,
) -> Result<TaskRef> {
    app.db.request_git_refresh(group_id, repository_key).await?;
    enqueue_refresh(app, group_id, repository_key).await
}

/// Admits one Git index task for a source, reusing an active task.
pub(crate) async fn enqueue_refresh(
    app: &Context69App,
    group_id: i64,
    repository_key: Uuid,
) -> Result<TaskRef> {
    let source = app
        .db
        .get_git_repository_source(group_id, repository_key)
        .await?
        .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;
    if !source.enabled || source.deletion_requested_at.is_some() {
        bail!(DomainError::conflict("git_repository_not_admitting_work"));
    }
    let user_id = app
        .db
        .resolve_git_refresh_actor(group_id, source.created_by_user_id)
        .await?
        .ok_or_else(|| anyhow!(DomainError::conflict("git_refresh_actor_unavailable")))?;
    app.db
        .mark_git_refresh_attempt(group_id, repository_key)
        .await?;
    app.tasks
        .submit(TaskSubmission {
            user_id,
            group_id: Some(group_id),
            group_path: Some(source.group.group_path.clone()),
            source_key: None,
            kind: TaskKind::GitIndex,
            payloads: vec![serde_json::json!({ "repository_key": repository_key })],
            input_storage_object_ids: Vec::new(),
            idempotency_key: None,
        })
        .await
}
