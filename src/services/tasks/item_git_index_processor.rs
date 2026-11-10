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

use crate::contracts::sources::{GitConnectionMode, GitProviderKind};
use crate::db::{ClaimedItem, Database, StoredGitRepositorySource};
use crate::domain::GroupRecord;
use crate::domain_errors::{DomainError, find_domain_error};
use crate::services::{
    git_repository::{
        ForgejoOrigin, GitHubApiTransport,
        code_vectors::CodeVectorRequest,
        indexing::{
            ForgejoSnapshotProvider, GitHubSnapshotProvider, GitSnapshotIndexOutcome,
            GitSnapshotProvider, index_git_repository_snapshot_at_revision,
        },
        limits::GitIndexLimits,
        provider_credentials::GitAccessCredential,
        provider_transport::{ProviderApiOrigin, ProviderApiTransport},
        provider_url,
    },
    git_secrets::{GitSecretReader, GitSecretSlot, GitSecretTarget},
};

use super::TaskService;
use super::item_processors::ProcessResult;

/// Stage that runs the bounded snapshot indexer; the kind's entry stage.
pub(super) const GIT_INDEX_STAGE: &str = "indexing";

/// Stage that only completes an item whose snapshot already activated.
const GIT_INDEX_FINALIZE: &str = "finalize";

/// Payload field carrying the repository key.
const REPOSITORY_KEY_FIELD: &str = "repository_key";

/// Payload field asking for a rebuild even when the target is unchanged.
const FORCE_REBUILD_FIELD: &str = "force_rebuild";

/// Largest failure text recorded on the item projection.
const MAX_ERROR_MESSAGE: usize = 512;

/// Separator between the stable failure code and the human-readable detail on
/// an internal `ProcessResult` message. The outer processor removes it before
/// the item projection is written.
const FAILURE_CODE_SEPARATOR: char = '\u{1f}';

/// What one `git_index` item run must do.
enum GitIndexPlan {
    Run {
        group_id: i64,
        repository_key: Uuid,
        force_rebuild: bool,
    },
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
    let force_rebuild = parse_force_rebuild(payload)
        .map_err(|error| terminal_failure(GIT_INDEX_STAGE, &error.to_string()))?;
    Ok(GitIndexPlan::Run {
        group_id,
        repository_key,
        force_rebuild,
    })
}

/// Reads the optional force-rebuild flag, defaulting to false.
fn parse_force_rebuild(payload: &Value) -> Result<bool> {
    match payload.get(FORCE_REBUILD_FIELD) {
        None => Ok(false),
        Some(Value::Bool(force)) => Ok(*force),
        Some(_) => Err(DomainError::invalid_argument("git_index_force_rebuild_invalid").into()),
    }
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
    let mut result = process_git_index_plan(service, item, plan).await?;
    if let (Some(group_id), Some(repository_key), Some(config_revision)) = (
        result.group_id,
        result.repository_key,
        result.config_revision,
    ) {
        record_terminal_failure(
            service.db(),
            group_id,
            repository_key,
            config_revision,
            &mut result.result,
        )
        .await;
    }
    Ok(result.result)
}

/// A processed result plus the source fence a terminal failure must respect.
struct GitIndexOutcome {
    result: ProcessResult,
    group_id: Option<i64>,
    repository_key: Option<Uuid>,
    config_revision: Option<i64>,
}

impl GitIndexOutcome {
    fn plain(result: ProcessResult) -> Self {
        Self {
            result,
            group_id: None,
            repository_key: None,
            config_revision: None,
        }
    }

    fn fenced(
        result: ProcessResult,
        group_id: i64,
        repository_key: Uuid,
        config_revision: i64,
    ) -> Self {
        Self {
            result,
            group_id: Some(group_id),
            repository_key: Some(repository_key),
            config_revision: Some(config_revision),
        }
    }
}

async fn process_git_index_plan(
    service: &TaskService,
    item: &mut ClaimedItem,
    plan: GitIndexPlan,
) -> Result<GitIndexOutcome> {
    let (group_id, repository_key, force_rebuild) = match plan {
        GitIndexPlan::Finalize => {
            return Ok(GitIndexOutcome::plain(finalize_result(&item.payload)));
        }
        GitIndexPlan::Run {
            group_id,
            repository_key,
            force_rebuild,
        } => (group_id, repository_key, force_rebuild),
    };
    let source = match service
        .db()
        .get_git_repository_source(group_id, repository_key)
        .await
    {
        Ok(Some(source)) => source,
        Ok(None) => {
            return Ok(GitIndexOutcome::plain(terminal_failure(
                GIT_INDEX_STAGE,
                "git_index_source_missing",
            )));
        }
        Err(error) => {
            return Ok(GitIndexOutcome::plain(index_failure(
                GIT_INDEX_STAGE,
                &error,
            )));
        }
    };
    let config_revision = source.config_revision;
    let observed_request = source
        .refresh_dirty
        .then_some(source.refresh_requested_at)
        .flatten();
    let retention_count = source.retention_count;
    let provider = match build_provider(service, &source).await {
        Ok(provider) => provider,
        Err(result) => {
            return Ok(GitIndexOutcome::fenced(
                result,
                group_id,
                repository_key,
                config_revision,
            ));
        }
    };
    let code = match service.settings().code_embedding_snapshot() {
        Some(snapshot) => {
            let qdrant_url = match service.settings().runtime_qdrant_url().await {
                Ok(url) => url,
                Err(error) => {
                    return Ok(GitIndexOutcome::fenced(
                        index_failure(GIT_INDEX_STAGE, &error),
                        group_id,
                        repository_key,
                        config_revision,
                    ));
                }
            };
            Some(CodeVectorRequest::production(snapshot, qdrant_url))
        }
        None => None,
    };
    let result = run_git_index_snapshot_with_code_at_revision(
        service.db(),
        group_id,
        repository_key,
        provider.as_ref(),
        GitIndexLimits::default(),
        code.as_ref(),
        force_rebuild,
        config_revision,
    )
    .await;
    let result = if matches!(&result, ProcessResult::Progressed { .. }) {
        let qdrant_url = code
            .as_ref()
            .and_then(|request| request.qdrant_url.as_deref());
        match crate::services::git_repository::refresh::complete_refresh(
            service.db(),
            group_id,
            repository_key,
            observed_request,
            retention_count,
            qdrant_url,
        )
        .await
        {
            Ok(()) => result,
            Err(error) => index_failure(GIT_INDEX_STAGE, &error),
        }
    } else {
        result
    };
    Ok(GitIndexOutcome::fenced(
        result,
        group_id,
        repository_key,
        config_revision,
    ))
}

/// Builds the provider implementation from the source snapshot the task read
/// before it captured the attempt revision.
///
/// The source owns the provider family and the optional connection reference,
/// so a task can never choose a different provider through its payload. Public
/// acquisition needs no credential. A token connection's read credential is
/// opened from the deployment secret store through the same record-scoped name
/// the writer sealed it under, and is held only in the in-memory credential
/// seam; it never enters the task payload, an error, or a log. GitHub and
/// Forgejo both bind their transport to the connection origin. Installation
/// mode is unsupported on either provider and fails closed. The snapshot is
/// never re-read here: the attempt is bound to the revision captured with it.
async fn build_provider(
    service: &TaskService,
    source: &StoredGitRepositorySource,
) -> std::result::Result<Box<dyn GitSnapshotProvider>, ProcessResult> {
    let group_id = source.group.group_id;
    let trusted_proxy = service
        .settings()
        .trusted_proxy_enabled()
        .await
        .map_err(|error| index_failure(GIT_INDEX_STAGE, &error))?;
    let connection = match source.connection_key.as_deref() {
        Some(key) => service
            .db()
            .get_git_provider_connection(group_id, key)
            .await
            .map_err(|error| index_failure(GIT_INDEX_STAGE, &error))?
            .ok_or_else(|| terminal_failure(GIT_INDEX_STAGE, "git_connection_not_found"))?,
        None => {
            if source.provider.requires_connection() {
                return Err(terminal_failure(GIT_INDEX_STAGE, "git_connection_required"));
            }
            let transport = GitHubApiTransport::new(trusted_proxy)
                .await
                .map_err(|error| index_failure(GIT_INDEX_STAGE, &error))?;
            return Ok(Box::new(GitHubSnapshotProvider::new(Arc::new(transport))));
        }
    };
    if connection.disabled_at.is_some() {
        return Err(terminal_failure(GIT_INDEX_STAGE, "git_connection_disabled"));
    }
    if connection.provider != source.provider {
        return Err(terminal_failure(
            GIT_INDEX_STAGE,
            "git_connection_provider_mismatch",
        ));
    }
    provider_url::validate_connection_origin(
        connection.provider,
        &connection.base_url,
        &source.canonical_url,
    )
    .map_err(|_| terminal_failure(GIT_INDEX_STAGE, "git_connection_origin_mismatch"))?;
    match source.provider {
        GitProviderKind::GitHub => {
            let credential = match connection.mode {
                GitConnectionMode::Token => Some(
                    read_connection_credential(service, group_id, &connection.connection_key)
                        .await?,
                ),
                GitConnectionMode::Public => None,
                GitConnectionMode::Installation => {
                    return Err(terminal_failure(
                        GIT_INDEX_STAGE,
                        "git_connection_mode_unsupported",
                    ));
                }
            };
            let transport = GitHubApiTransport::with_credential(trusted_proxy, credential)
                .await
                .map_err(|error| index_failure(GIT_INDEX_STAGE, &error))?;
            Ok(Box::new(GitHubSnapshotProvider::new(Arc::new(transport))))
        }
        GitProviderKind::Forgejo => {
            let credential = match connection.mode {
                GitConnectionMode::Token => Some(
                    read_connection_credential(service, group_id, &connection.connection_key)
                        .await?,
                ),
                GitConnectionMode::Public => None,
                GitConnectionMode::Installation => {
                    return Err(terminal_failure(
                        GIT_INDEX_STAGE,
                        "git_connection_mode_unsupported",
                    ));
                }
            };
            let origin = ForgejoOrigin::from_base_url(&connection.base_url)
                .map_err(|_| terminal_failure(GIT_INDEX_STAGE, "git_connection_origin_invalid"))?;
            let api_origin = ProviderApiOrigin::parse(&origin.api_base())
                .map_err(|_| terminal_failure(GIT_INDEX_STAGE, "git_connection_origin_invalid"))?;
            let transport = ProviderApiTransport::new(trusted_proxy, api_origin, credential)
                .await
                .map_err(|error| index_failure(GIT_INDEX_STAGE, &error))?;
            Ok(Box::new(ForgejoSnapshotProvider::new(
                Arc::new(transport),
                origin,
            )))
        }
        _ => Err(terminal_failure(
            GIT_INDEX_STAGE,
            "git_provider_unsupported",
        )),
    }
}

/// Opens one connection's sealed read credential through the group-qualified
/// record target. A missing or unreadable credential is terminal; it never
/// falls back to an anonymous request.
async fn read_connection_credential(
    service: &TaskService,
    group_id: i64,
    connection_key: &str,
) -> std::result::Result<GitAccessCredential, ProcessResult> {
    let secret = GitSecretReader::new(service.secrets().clone())
        .read(
            &GitSecretTarget::Connection {
                group_id,
                connection_key: connection_key.to_string(),
            },
            GitSecretSlot::Credential,
        )
        .await
        .map_err(|_| terminal_failure(GIT_INDEX_STAGE, "git_connection_credential_unavailable"))?;
    let Some(secret) = secret else {
        return Err(terminal_failure(
            GIT_INDEX_STAGE,
            "git_connection_credential_missing",
        ));
    };
    Ok(GitAccessCredential::new(secret))
}

async fn run_git_index_snapshot_with_code_at_revision(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    provider: &dyn GitSnapshotProvider,
    limits: GitIndexLimits,
    code: Option<&CodeVectorRequest>,
    force_rebuild: bool,
    expected_config_revision: i64,
) -> ProcessResult {
    match index_git_repository_snapshot_at_revision(
        db,
        provider,
        limits,
        group_id,
        repository_key,
        code,
        force_rebuild,
        expected_config_revision,
    )
    .await
    {
        Ok(outcome) => snapshot_progressed(outcome),
        Err(error) => index_failure(GIT_INDEX_STAGE, &error),
    }
}

fn snapshot_progressed(outcome: GitSnapshotIndexOutcome) -> ProcessResult {
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

/// Test-only lexical entry point retained so the existing processor tests keep
/// exercising the same five-argument call without a code request.
#[cfg(test)]
async fn run_git_index_snapshot(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    provider: &dyn GitSnapshotProvider,
    limits: GitIndexLimits,
) -> ProcessResult {
    match super::super::git_repository::indexing::index_git_repository_snapshot(
        db,
        provider,
        limits,
        group_id,
        repository_key,
        None,
        false,
    )
    .await
    {
        Ok(outcome) => snapshot_progressed(outcome),
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
    let code = if safe_failure_code(message) {
        message
    } else {
        "git_index_terminal_failure"
    };
    failure_result(stage, code, message, false)
}

/// Maps one indexer failure onto the existing retry/terminal semantics.
///
/// Only genuinely transient failures retry, and the worker's attempt cap
/// bounds them; every other typed failure is terminal. An untyped failure is
/// infrastructure (database or transport) and retries within the same cap.
fn index_failure(stage: &str, error: &anyhow::Error) -> ProcessResult {
    failure_result(
        stage,
        &failure_code(error),
        &error.to_string(),
        is_bounded_retry(error),
    )
}

fn failure_result(stage: &str, code: &str, detail: &str, retryable: bool) -> ProcessResult {
    ProcessResult::Failed {
        stage: stage.to_string(),
        message: bounded(format!("{code}{FAILURE_CODE_SEPARATOR}{detail}")),
        retryable,
    }
}

/// Persists the bounded code of a terminal failure and strips the internal
/// separator from the item message.
async fn record_terminal_failure(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    config_revision: i64,
    result: &mut ProcessResult,
) {
    let ProcessResult::Failed {
        retryable: false,
        message,
        ..
    } = result
    else {
        return;
    };
    let Some((code, detail)) = message.split_once(FAILURE_CODE_SEPARATOR) else {
        return;
    };
    let code = code.to_string();
    let detail = detail.to_string();
    if let Err(error) = db
        .mark_git_repository_failure(group_id, repository_key, config_revision, &code)
        .await
    {
        tracing::warn!(%error, %repository_key, "git index failure code could not be recorded");
    }
    *message = detail;
}

/// A stable, non-secret classification of one indexer error.
///
/// Domain messages that already use the bounded code alphabet are preserved;
/// everything else falls back to a variant category, so a provider message,
/// response body, or path can never reach the source projection.
fn failure_code(error: &anyhow::Error) -> String {
    let Some(domain) = find_domain_error(error) else {
        return "git_index_failed".to_string();
    };
    let message = match domain {
        DomainError::InvalidArgument(message)
        | DomainError::Unauthorized(message)
        | DomainError::Forbidden(message)
        | DomainError::NotFound(message)
        | DomainError::Conflict(message)
        | DomainError::PayloadTooLarge(message)
        | DomainError::UnprocessableEntity(message)
        | DomainError::RateLimited(message)
        | DomainError::Unavailable(message)
        | DomainError::UpstreamError(message)
        | DomainError::UpstreamTimeout(message)
        | DomainError::Internal(message) => message,
    };
    if safe_failure_code(message) {
        return message.clone();
    }
    match domain {
        DomainError::InvalidArgument(_) => "git_index_invalid_argument",
        DomainError::Unauthorized(_) => "git_index_unauthorized",
        DomainError::Forbidden(_) => "git_index_forbidden",
        DomainError::NotFound(_) => "git_index_not_found",
        DomainError::Conflict(_) => "git_index_conflict",
        DomainError::PayloadTooLarge(_) => "git_index_payload_too_large",
        DomainError::UnprocessableEntity(_) => "git_index_unprocessable",
        DomainError::RateLimited(_) => "git_index_rate_limited",
        DomainError::Unavailable(_) => "git_index_unavailable",
        DomainError::UpstreamError(_) => "git_index_upstream_error",
        DomainError::UpstreamTimeout(_) => "git_index_upstream_timeout",
        DomainError::Internal(_) => "git_index_internal",
    }
    .to_string()
}

fn safe_failure_code(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && value.len() <= 64
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

/// The shared retry policy, with the indexer's own rule for an inconsistent
/// repository snapshot.
///
/// A repository manifest that cannot be read is a terminal inconsistency of
/// the snapshot, not a transport failure, so an internal error stays terminal
/// here even though the shared policy treats an unknown internal failure as
/// transient everywhere else.
fn is_bounded_retry(error: &anyhow::Error) -> bool {
    if matches!(find_domain_error(error), Some(DomainError::Internal(_))) {
        return false;
    }
    crate::retry_policy::is_transient(error)
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
