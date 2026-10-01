//! Durable Git snapshot index task processor (issue #681 work unit 3C1).
//!
//! The existing task worker drives a `git_index` item through the collapsed
//! stage machine. The `indexing` stage parses the bounded repository-key
//! payload, fails closed when the group, payload, or repository is missing or
//! foreign, and runs the 3B3 core snapshot indexer with the configured
//! trusted-proxy setting and a fresh default [`GitIndexLimits`]. Success
//! advances to `finalize`, which only completes the item; typed indexer
//! failures map onto the existing retry/terminal task semantics so a permanent
//! acquisition failure never retries forever.
//!
//! The repository row already owns the canonical URL, ref, and policies, so
//! the payload is bounded to the repository key alone: a task can never smuggle
//! a provider target through item payload.

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

use crate::db::{ClaimedItem, Database};
use crate::domain::GroupRecord;
use crate::domain_errors::{DomainError, find_domain_error};
use crate::services::git_repository::{
    GitHubApiTransport,
    index_generation::{
        GitHubSnapshotProvider, GitSnapshotProvider, index_git_repository_snapshot,
    },
    limits::GitIndexLimits,
};

use super::TaskService;
use super::item_processors::ProcessResult;

/// Stage that runs the bounded snapshot indexer; the kind's entry stage.
pub(super) const GIT_INDEX_STAGE: &str = "indexing";

/// Stage that only completes an item whose snapshot already activated.
const GIT_INDEX_FINALIZE: &str = "finalize";

/// Payload field carrying the repository key.
const REPOSITORY_KEY_FIELD: &str = "repository_key";

/// Largest failure text recorded on the item projection.
const MAX_ERROR_MESSAGE: usize = 512;

/// What one `git_index` item run must do.
enum GitIndexPlan {
    Run { group_id: i64, repository_key: Uuid },
    Finalize,
}

/// Validates the stage, group, and repository-key payload without touching the
/// network, so a malformed or foreign request fails closed before any provider
/// work. `Err` carries the terminal result the item must take.
fn plan_git_index(
    group_id: Option<i64>,
    payload: &Value,
    stage: &str,
) -> std::result::Result<GitIndexPlan, ProcessResult> {
    if stage == GIT_INDEX_FINALIZE {
        return Ok(GitIndexPlan::Finalize);
    }
    if stage != GIT_INDEX_STAGE {
        return Err(terminal_failure(
            stage,
            &format!("unsupported git index task stage {stage}"),
        ));
    }
    let Some(group_id) = group_id else {
        return Err(terminal_failure(
            GIT_INDEX_STAGE,
            "git_index_requires_group",
        ));
    };
    let repository_key = parse_repository_key(payload)
        .map_err(|error| terminal_failure(GIT_INDEX_STAGE, &error.to_string()))?;
    Ok(GitIndexPlan::Run {
        group_id,
        repository_key,
    })
}

/// Reads the single bounded repository key from the item payload.
fn parse_repository_key(payload: &Value) -> Result<Uuid> {
    let value = payload
        .get(REPOSITORY_KEY_FIELD)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow::Error::from(DomainError::invalid_argument(
                "git_index_repository_key_missing",
            ))
        })?;
    Uuid::parse_str(value.trim())
        .map_err(|_| DomainError::invalid_argument("git_index_repository_key_invalid").into())
}

pub(super) async fn process_git_index(
    service: &TaskService,
    group: Option<&GroupRecord>,
    item: &mut ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let plan = match plan_git_index(group.map(|group| group.id), &item.payload, stage) {
        Ok(plan) => plan,
        Err(result) => return Ok(result),
    };
    let (group_id, repository_key) = match plan {
        GitIndexPlan::Finalize => return Ok(finalize_result(&item.payload)),
        GitIndexPlan::Run {
            group_id,
            repository_key,
        } => (group_id, repository_key),
    };
    let provider = match build_provider(service).await {
        Ok(provider) => provider,
        Err(result) => return Ok(result),
    };
    Ok(run_git_index_snapshot(
        service.db(),
        group_id,
        repository_key,
        provider.as_ref(),
        GitIndexLimits::default(),
    )
    .await)
}

/// Builds the credential-free public GitHub provider with the configured
/// trusted-proxy setting. A transport build failure is classified like any
/// other acquisition failure instead of aborting the worker.
async fn build_provider(
    service: &TaskService,
) -> std::result::Result<Box<dyn GitSnapshotProvider>, ProcessResult> {
    let trusted_proxy = service
        .settings()
        .trusted_proxy_enabled()
        .await
        .map_err(|error| index_failure(GIT_INDEX_STAGE, &error))?;
    let transport = GitHubApiTransport::new(trusted_proxy)
        .await
        .map_err(|error| index_failure(GIT_INDEX_STAGE, &error))?;
    Ok(Box::new(GitHubSnapshotProvider::new(Arc::new(transport))))
}

async fn run_git_index_snapshot(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    provider: &dyn GitSnapshotProvider,
    limits: GitIndexLimits,
) -> ProcessResult {
    match index_git_repository_snapshot(db, provider, limits, group_id, repository_key).await {
        Ok(outcome) => {
            tracing::info!(
                repository_key = %outcome.repository_key,
                generation = outcome.generation_number,
                commit_sha = %outcome.commit_sha,
                file_count = outcome.file_count,
                excluded_file_count = outcome.excluded_file_count,
                total_bytes = outcome.total_bytes,
                "git repository snapshot indexed"
            );
            ProcessResult::Progressed {
                next: GIT_INDEX_FINALIZE,
            }
        }
        Err(error) => index_failure(GIT_INDEX_STAGE, &error),
    }
}

fn finalize_result(payload: &Value) -> ProcessResult {
    ProcessResult::Succeeded(
        parse_repository_key(payload)
            .ok()
            .map(|key| key.to_string()),
    )
}

fn terminal_failure(stage: &str, message: &str) -> ProcessResult {
    ProcessResult::Failed {
        stage: stage.to_string(),
        message: bounded(message.to_owned()),
        retryable: false,
    }
}

/// Maps one indexer failure onto the existing retry/terminal semantics.
///
/// Only genuinely transient failures retry, and the worker's attempt cap
/// bounds them; every other typed failure is terminal. An untyped failure is
/// infrastructure (database or transport) and retries within the same cap.
fn index_failure(stage: &str, error: &anyhow::Error) -> ProcessResult {
    ProcessResult::Failed {
        stage: stage.to_string(),
        message: bounded(error.to_string()),
        retryable: is_bounded_retry(error),
    }
}

fn is_bounded_retry(error: &anyhow::Error) -> bool {
    match find_domain_error(error) {
        Some(
            DomainError::UpstreamError(_)
            | DomainError::UpstreamTimeout(_)
            | DomainError::RateLimited(_)
            | DomainError::Unavailable(_)
            | DomainError::Conflict(_),
        ) => true,
        Some(_) => false,
        None => true,
    }
}

fn bounded(message: String) -> String {
    if message.chars().count() <= MAX_ERROR_MESSAGE {
        return message;
    }
    message.chars().take(MAX_ERROR_MESSAGE).collect()
}

#[cfg(test)]
#[path = "item_git_index_processor_tests.rs"]
mod tests;
