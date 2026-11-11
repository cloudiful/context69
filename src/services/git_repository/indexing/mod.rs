//! Bounded initial snapshot indexing (issue #681 work unit 3B3).
//!
//! One call indexes a single bounded public snapshot into a new group-owned
//! generation: it resolves and pins the ref, marks the source indexing, opens a
//! building generation, walks the bounded tree under explicit eligible-file and
//! Blob-fetch counts, fetches and classifies each eligible blob, writes the
//! complete manifest and its chunks, re-reads the source to reject a changed
//! target as stale, and only then activates the generation and advances the
//! indexed checkpoint.
//!
//! Nothing is activated before every included file succeeds. On any failure the
//! partial content is retired and the generation is marked failed (best effort),
//! so the previous active generation keeps serving. An empty eligible snapshot
//! completes safely. Acquisition stays provider-neutral behind an in-crate
//! factory that builds a fresh acquirer per generation, so aggregate byte
//! budgets are never reused, and public acquisition carries no credential.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::code_vectors::CodeVectorRequest;
use super::forgejo_client::ForgejoAcquirer;
use super::github_client::{GitHubAcquirer, RepositoryAcquirer};
use super::http::GitHttpTransport;
use super::limits::GitIndexLimits;
use super::model::GitAcquisitionLimits;
use super::provider_url::ForgejoOrigin;
use crate::contracts::sources::GitRepositorySkipSummary;
use crate::db::Database;

mod manifest;
mod rollback;
mod snapshot;

use rollback::{SnapshotProgress, fail_snapshot};
use snapshot::attempt_snapshot;

/// Version of the pure content pipeline (eligibility rules plus chunker) that
/// produced a manifest. A behavior change to selection or structural chunking
/// must bump it so a persisted pipeline identity invalidates old builds instead
/// of mixing chunk shapes from different pipelines.
pub(crate) const GIT_INDEX_PIPELINE_VERSION: u32 = 2;

/// What one completed snapshot indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitSnapshotIndexOutcome {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub generation_number: i64,
    pub ref_name: String,
    pub commit_sha: String,
    pub file_count: i64,
    pub excluded_file_count: i64,
    pub total_bytes: i64,
    pub activated_at: DateTime<Utc>,
    pub pipeline_version: u32,
    /// The target commit and pipeline identity already matched the active
    /// generation, so no new generation was written.
    pub no_op: bool,
    pub reused_file_count: i64,
    pub changed_file_count: i64,
    pub skip_summary: GitRepositorySkipSummary,
}

/// Builds an acquirer for one snapshot.
///
/// The factory is called once per generation, so every snapshot gets its own
/// aggregate byte budget instead of sharing the spend of a previous walk. A
/// provider implementation that authenticates is free to do so, but public
/// acquisition never requires a credential.
pub(crate) trait GitSnapshotProvider: Send + Sync {
    fn acquirer(
        &self,
        canonical_url: &str,
        limits: GitAcquisitionLimits,
    ) -> Result<Box<dyn RepositoryAcquirer>>;
}

/// GitHub provider built on the bounded 3A client. A public transport stays
/// anonymous; a token transport carries its optional PAT as an origin-bound
/// `Bearer` header, so the provider itself never sees the credential.
pub(crate) struct GitHubSnapshotProvider<T: GitHttpTransport + ?Sized = dyn GitHttpTransport> {
    transport: Arc<T>,
}

impl<T: GitHttpTransport + ?Sized> GitHubSnapshotProvider<T> {
    pub(crate) fn new(transport: Arc<T>) -> Self {
        Self { transport }
    }
}

impl<T: GitHttpTransport + ?Sized + Sync + 'static> GitSnapshotProvider
    for GitHubSnapshotProvider<T>
{
    fn acquirer(
        &self,
        canonical_url: &str,
        limits: GitAcquisitionLimits,
    ) -> Result<Box<dyn RepositoryAcquirer>> {
        Ok(Box::new(GitHubAcquirer::new(
            canonical_url,
            Arc::clone(&self.transport),
            limits,
        )?))
    }
}

/// Forgejo provider bound to one validated instance origin.
///
/// The origin is established by the attached connection, so a repository URL
/// can only resolve inside that origin. Credential loading happens before the
/// provider is built and is not part of this type.
pub(crate) struct ForgejoSnapshotProvider<T: GitHttpTransport + ?Sized = dyn GitHttpTransport> {
    transport: Arc<T>,
    origin: ForgejoOrigin,
}

impl<T: GitHttpTransport + ?Sized> ForgejoSnapshotProvider<T> {
    pub(crate) fn new(transport: Arc<T>, origin: ForgejoOrigin) -> Self {
        Self { transport, origin }
    }
}

impl<T: GitHttpTransport + ?Sized + Sync + 'static> GitSnapshotProvider
    for ForgejoSnapshotProvider<T>
{
    fn acquirer(
        &self,
        canonical_url: &str,
        limits: GitAcquisitionLimits,
    ) -> Result<Box<dyn RepositoryAcquirer>> {
        Ok(Box::new(ForgejoAcquirer::new(
            self.origin.clone(),
            canonical_url,
            Arc::clone(&self.transport),
            limits,
        )?))
    }
}

/// Indexes one snapshot against the source revision the caller captured before
/// building the provider.
///
/// The attempt may only begin and publish while that captured revision is still
/// current, so a connection attachment, disable, or provider-connection change
/// that happened after provider construction cannot be overwritten by network
/// work that used the earlier credential.
pub(crate) async fn index_git_repository_snapshot_at_revision(
    db: &Database,
    provider: &dyn GitSnapshotProvider,
    limits: GitIndexLimits,
    group_id: i64,
    repository_key: Uuid,
    code: Option<&CodeVectorRequest>,
    force_rebuild: bool,
    expected_config_revision: i64,
) -> Result<GitSnapshotIndexOutcome> {
    let guard = crate::db::GitIndexLifetimeGuard::acquire(db.pool(), repository_key).await?;
    let mut progress = SnapshotProgress::default();
    let result = match attempt_snapshot(
        db,
        provider,
        &limits,
        group_id,
        repository_key,
        code,
        force_rebuild,
        expected_config_revision,
        &mut progress,
    )
    .await
    {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            fail_snapshot(db, group_id, repository_key, progress).await;
            Err(error)
        }
    };
    if let Err(error) = guard.release().await {
        tracing::warn!(%repository_key, %error, "git index lifetime lock release failed");
    }
    result
}
