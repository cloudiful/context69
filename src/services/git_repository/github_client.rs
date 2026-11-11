//! Bounded public GitHub tree/blob client (issue #681 phase 3A).
//!
//! The client resolves a ref to a commit SHA, lists one recursive tree, and
//! fetches individual blob contents from the public GitHub REST API. It
//! never executes repository content, shells out to Git, follows redirects,
//! attaches credentials, or reflects response bodies into errors. Every
//! response is checked against [`GitAcquisitionLimits`] before bytes are
//! handed out: entry counts, per-blob and aggregate decoded bytes, declared
//! sizes, and tree truncation all fail closed.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;

use crate::domain_errors::DomainError;

use super::bounds::{TREE_ENTRY_BLOB_TYPE, check_declared_length};
use super::github_schemas::{
    BlobResponse, RefResponse, RepoMetadataResponse, TagResponse, TreeResponse,
};
use super::github_url::{
    GitHubRepoCoordinates, blob_endpoint, parse_canonical_github_url, ref_endpoint,
    repo_metadata_endpoint, tag_endpoint, tree_endpoint,
};
use super::http::{GitHttpTransport, decode_blob_content, get_bounded, parse_json};
use super::model::{BlobContent, GitAcquisitionLimits, TreeFileEntry, TreeListing};
use super::ref_path_safety::{SafeRef, SafeSha, SafeTreePath};
use super::resolve::{MAX_TAG_DEREF_DEPTH, validated_default_branch};

/// Git file mode for symlinks; their blobs are link targets, not content.
const SYMLINK_MODE: &str = "120000";

/// Provider-neutral acquisition surface. Later source/task/index phases
/// build scheduling and persistence on top of this trait without depending
/// on a concrete provider client.
#[async_trait]
pub(crate) trait RepositoryAcquirer: Send + Sync {
    /// Resolve a ref (`refs/heads/main`, `HEAD`) to its commit SHA.
    async fn resolve_ref(&self, safe_ref: &SafeRef) -> Result<SafeSha>;

    /// List the recursive tree of `commit_sha` and return its file entries
    /// pinned to that commit. `commit_sha` is a commit identity, not a tree
    /// id; the provider response is validated against it before entries are
    /// accepted.
    async fn list_tree(&self, commit_sha: &SafeSha) -> Result<TreeListing>;

    /// Fetch and decode one blob, bounded by per-blob and aggregate limits.
    async fn fetch_blob(&self, blob_sha: &SafeSha) -> Result<BlobContent>;
}

/// Public GitHub implementation of [`RepositoryAcquirer`].
pub(crate) struct GitHubAcquirer<T: GitHttpTransport + ?Sized = dyn GitHttpTransport> {
    transport: std::sync::Arc<T>,
    coordinates: GitHubRepoCoordinates,
    limits: GitAcquisitionLimits,
    /// Decoded bytes charged across the lifetime of this client. The trait
    /// hands out `&self`, so the aggregate counter is atomic.
    total_spent: std::sync::atomic::AtomicUsize,
}

impl<T: GitHttpTransport + ?Sized> GitHubAcquirer<T> {
    /// Bind a client to one canonical repository through the shared
    /// strict URL parser, so endpoint construction can never widen the
    /// approved host or path shape.
    pub(crate) fn new(
        canonical_url: &str,
        transport: std::sync::Arc<T>,
        limits: GitAcquisitionLimits,
    ) -> Result<Self> {
        Ok(Self {
            transport,
            coordinates: parse_canonical_github_url(canonical_url)?,
            limits,
            total_spent: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Core ref resolution, shared by the trait impl and the resolution
    /// flows: `HEAD` routes through repository `default_branch` metadata,
    /// annotated tags dereference to commits with bounded depth.
    async fn resolve_ref_inner(&self, safe_ref: &SafeRef) -> Result<SafeSha> {
        if safe_ref.as_str() == "HEAD" {
            let url = repo_metadata_endpoint(&self.coordinates);
            let response = get_bounded(
                self.transport.as_ref(),
                url.as_str(),
                self.limits.max_response_bytes,
            )
            .await?;
            let metadata: RepoMetadataResponse = parse_json(&response)?;
            let default_ref = validated_default_branch(&metadata.default_branch)?;
            return Box::pin(self.resolve_ref_inner(&default_ref)).await;
        }
        let url = ref_endpoint(&self.coordinates, safe_ref);
        let response = get_bounded(
            self.transport.as_ref(),
            url.as_str(),
            self.limits.max_response_bytes,
        )
        .await?;
        let parsed: RefResponse = parse_json(&response)?;
        match parsed.object.kind.as_str() {
            "commit" => SafeSha::parse(&parsed.object.sha),
            // Annotated tags: the ref points at a tag object, not a commit.
            "tag" => {
                let tag_sha = SafeSha::parse(&parsed.object.sha)?;
                self.dereference_tag(&tag_sha).await
            }
            _ => Err(anyhow!(DomainError::upstream_error("git_ref_not_commit"))),
        }
    }

    /// Follow one annotated tag (`git/tags/{sha}`) to its target.
    async fn tag_target(&self, sha: &SafeSha) -> Result<(SafeSha, String)> {
        let url = tag_endpoint(&self.coordinates, sha.as_str());
        let response = get_bounded(
            self.transport.as_ref(),
            url.as_str(),
            self.limits.max_response_bytes,
        )
        .await?;
        let parsed: TagResponse = parse_json(&response)?;
        if SafeSha::parse(&parsed.sha)?.as_str() != sha.as_str() {
            return Err(anyhow!(DomainError::upstream_error("git_tag_sha_mismatch")));
        }
        Ok((SafeSha::parse(&parsed.target.sha)?, parsed.target.kind))
    }

    /// Dereference an annotated tag (or tag chain) down to its commit with
    /// bounded depth, so a hostile repository cannot turn one resolution
    /// into an unbounded request walk.
    async fn dereference_tag(&self, tag_sha: &SafeSha) -> Result<SafeSha> {
        let mut current = tag_sha.clone();
        for _ in 0..MAX_TAG_DEREF_DEPTH {
            let (target, kind) = self.tag_target(&current).await?;
            match kind.as_str() {
                // The chain ends at a commit: dereferencing is complete.
                "commit" => return SafeSha::parse(target.as_str()),
                "tag" => current = target,
                // Trees and blobs are not resolvable commits.
                _ => {
                    return Err(anyhow!(DomainError::upstream_error("git_tag_not_commit")));
                }
            }
        }
        Err(anyhow!(DomainError::upstream_error(
            "git_tag_chain_too_deep"
        )))
    }
}

#[async_trait]
impl<T: GitHttpTransport + ?Sized + Sync> RepositoryAcquirer for GitHubAcquirer<T> {
    async fn resolve_ref(&self, safe_ref: &SafeRef) -> Result<SafeSha> {
        self.resolve_ref_inner(safe_ref).await
    }

    /// List the recursive tree of `commit_sha`.
    ///
    /// The provider echoes the requested commit id, so the echoed SHA is
    /// validated against the commit identity here — the same response-identity
    /// check the tag and blob legs perform — before any entry is trusted.
    async fn list_tree(&self, commit_sha: &SafeSha) -> Result<TreeListing> {
        let url = tree_endpoint(&self.coordinates, commit_sha.as_str());
        let response = get_bounded(
            self.transport.as_ref(),
            url.as_str(),
            self.limits.max_response_bytes,
        )
        .await?;
        let parsed: TreeResponse = parse_json(&response)?;
        if parsed.sha != commit_sha.as_str() {
            return Err(anyhow!(DomainError::upstream_error(
                "git_tree_commit_mismatch"
            )));
        }
        if parsed.truncated {
            return Err(anyhow!(DomainError::upstream_error("git_tree_truncated")));
        }
        if parsed.tree.len() > self.limits.max_tree_entries {
            return Err(DomainError::payload_too_large("git_tree_too_large").into());
        }
        let mut files = Vec::with_capacity(parsed.tree.len());
        let mut excluded_files = 0usize;
        let mut too_large_files = 0usize;
        for entry in parsed.tree {
            if entry.kind != TREE_ENTRY_BLOB_TYPE {
                continue;
            }
            // Symlink blobs (mode `120000`) contain a link target, not file
            // content, so they are excluded like directories — but counted,
            // because the provider did report a file entry here.
            if entry.mode.as_deref() == Some(SYMLINK_MODE) {
                excluded_files += 1;
                continue;
            }
            let declared_size = entry.size.ok_or_else(|| {
                anyhow!(DomainError::upstream_error("git_tree_entry_missing_size"))
            })?;
            // A declared size the per-blob budget can never serve excludes
            // only its own entry: fetching it would fail later anyway.
            if check_declared_length(Some(declared_size), self.limits.blob_budget()).is_err() {
                excluded_files += 1;
                too_large_files += 1;
                continue;
            }
            // A path that fails the safety classification excludes only its
            // own entry, never the whole listing — and is counted.
            let Ok(path) = SafeTreePath::parse(&entry.path) else {
                excluded_files += 1;
                continue;
            };
            files.push(TreeFileEntry {
                path,
                blob_sha: SafeSha::parse(&entry.sha)?,
                size: Some(declared_size),
            });
        }
        Ok(TreeListing {
            files,
            excluded_files,
            too_large_files,
        })
    }

    async fn fetch_blob(&self, blob_sha: &SafeSha) -> Result<BlobContent> {
        let url = blob_endpoint(&self.coordinates, blob_sha.as_str());
        let response = get_bounded(
            self.transport.as_ref(),
            url.as_str(),
            self.limits.max_response_bytes,
        )
        .await?;
        let parsed: BlobResponse = parse_json(&response)?;
        if parsed.encoding != "base64" {
            return Err(anyhow!(DomainError::invalid_argument("git_blob_encoding")));
        }
        let declared_size = parsed
            .size
            .ok_or_else(|| anyhow!(DomainError::upstream_error("git_blob_missing_size")))?;
        check_declared_length(Some(declared_size), self.limits.blob_budget())?;
        if SafeSha::parse(&parsed.sha)?.as_str() != blob_sha.as_str() {
            return Err(anyhow!(DomainError::upstream_error(
                "git_blob_sha_mismatch"
            )));
        }
        let decoded = decode_blob_content(&parsed.content, self.limits.max_blob_bytes)?;
        if decoded.len() as u64 != declared_size {
            return Err(anyhow!(DomainError::upstream_error(
                "git_blob_size_mismatch"
            )));
        }
        // Reserve the decoded bytes atomically: `fetch_update` retries on
        // contention, so concurrent fetches can never lose a charge and
        // slip past `max_total_bytes`.
        use std::sync::atomic::Ordering;
        let reserve = decoded.len();
        loop {
            let current = self.total_spent.load(Ordering::Relaxed);
            if current.saturating_add(reserve) > self.limits.max_total_bytes {
                return Err(DomainError::payload_too_large("git_aggregate_limit").into());
            }
            match self.total_spent.compare_exchange_weak(
                current,
                current + reserve,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
        Ok(BlobContent {
            bytes: Bytes::from(decoded),
        })
    }
}
