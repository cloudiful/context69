//! Bounded Forgejo/Gitea tree/blob client.
//!
//! Acquisition is origin-bound: the client is constructed with the validated
//! instance origin of the connection that owns the repository, and every
//! endpoint is built from that origin. Refs resolve through the refs list (the
//! Forgejo endpoint returns a list, not one object), annotated tags dereference
//! with bounded depth, and the recursive tree is read page by page until
//! distinct immutable-tree coverage reaches the declared total. A provider's
//! trailing `truncated` flag alone never forces another page, and an explicit
//! null tree is the empty branch. Pagination fails closed: a progress guard, an
//! entry budget, and an observed page budget all refuse a listing that cannot
//! prove complete coverage instead of returning a partial tree.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;

use crate::domain_errors::DomainError;

use super::bounds::{BoundedBase64, ByteBudget, TREE_ENTRY_BLOB_TYPE, check_declared_length};
use super::forgejo_schemas::{
    AnnotatedTag, GitBlobResponse, GitTreeResponse, Reference, RepositoryResponse,
};
use super::github_client::RepositoryAcquirer;
use super::github_transport::{GitHttpTransport, TransportResponse};
use super::model::{BlobContent, GitAcquisitionLimits, TreeFileEntry, TreeListing};
use super::provider_url::{
    ForgejoOrigin, ForgejoRepoCoordinates, forgejo_blob_endpoint, forgejo_refs_endpoint,
    forgejo_repo_endpoint, forgejo_tag_endpoint, forgejo_tree_endpoint,
};
use super::ref_path_safety::{SafeRef, SafeSha, SafeTreePath};
use super::resolve::{MAX_TAG_DEREF_DEPTH, validated_default_branch};

const SYMLINK_MODE: &str = "120000";
const TREE_PAGE_SIZE: u32 = 1000;
const MAX_TREE_PAGES: u32 = 1000;

/// Forgejo implementation of [`RepositoryAcquirer`].
pub(crate) struct ForgejoAcquirer<T: GitHttpTransport + ?Sized = dyn GitHttpTransport> {
    transport: std::sync::Arc<T>,
    origin: ForgejoOrigin,
    coordinates: ForgejoRepoCoordinates,
    limits: GitAcquisitionLimits,
    total_spent: std::sync::atomic::AtomicUsize,
}

impl<T: GitHttpTransport + ?Sized> ForgejoAcquirer<T> {
    pub(crate) fn new(
        origin: ForgejoOrigin,
        canonical_url: &str,
        transport: std::sync::Arc<T>,
        limits: GitAcquisitionLimits,
    ) -> Result<Self> {
        Ok(Self {
            transport,
            coordinates: origin.parse_repository_url(canonical_url)?,
            origin,
            limits,
            total_spent: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    async fn resolve_ref_inner(&self, safe_ref: &SafeRef) -> Result<SafeSha> {
        if safe_ref.as_str() == "HEAD" {
            let url = forgejo_repo_endpoint(&self.origin, &self.coordinates);
            let response = self.get_bounded(&url).await?;
            let metadata: RepositoryResponse = self.parse_json(&response)?;
            let default_ref = validated_default_branch(&metadata.default_branch)?;
            return Box::pin(self.resolve_ref_inner(&default_ref)).await;
        }
        let url = forgejo_refs_endpoint(&self.origin, &self.coordinates, safe_ref);
        let response = self.get_bounded(&url).await?;
        let parsed: Vec<Reference> = self.parse_json(&response)?;
        let reference = select_reference(parsed, safe_ref)?;
        match reference.object.kind.as_str() {
            "commit" => SafeSha::parse(&reference.object.sha),
            "tag" => {
                let tag_sha = SafeSha::parse(&reference.object.sha)?;
                self.dereference_tag(&tag_sha).await
            }
            _ => Err(anyhow!(DomainError::upstream_error("git_ref_not_commit"))),
        }
    }

    async fn tag_target(&self, sha: &SafeSha) -> Result<(SafeSha, String)> {
        let url = forgejo_tag_endpoint(&self.origin, &self.coordinates, sha.as_str());
        let response = self.get_bounded(&url).await?;
        let parsed: AnnotatedTag = self.parse_json(&response)?;
        if SafeSha::parse(&parsed.sha)?.as_str() != sha.as_str() {
            return Err(anyhow!(DomainError::upstream_error("git_tag_sha_mismatch")));
        }
        Ok((SafeSha::parse(&parsed.target.sha)?, parsed.target.kind))
    }

    async fn dereference_tag(&self, tag_sha: &SafeSha) -> Result<SafeSha> {
        let mut current = tag_sha.clone();
        for _ in 0..MAX_TAG_DEREF_DEPTH {
            let (target, kind) = self.tag_target(&current).await?;
            match kind.as_str() {
                "commit" => return SafeSha::parse(target.as_str()),
                "tag" => current = target,
                _ => {
                    return Err(anyhow!(DomainError::upstream_error("git_tag_not_commit")));
                }
            }
        }
        Err(anyhow!(DomainError::upstream_error(
            "git_tag_chain_too_deep"
        )))
    }

    fn decode_blob_content(&self, encoded: &str) -> Result<Vec<u8>> {
        let decoder_budget = ByteBudget {
            max_bytes: self
                .limits
                .max_blob_bytes
                .saturating_mul(7)
                .saturating_div(3)
                .saturating_add(8),
        };
        let mut decoder = BoundedBase64::new(decoder_budget);
        let mut decoded = decoder.push_chunk(encoded.as_bytes())?;
        decoded.extend(decoder.finish()?);
        Ok(decoded)
    }

    async fn get_bounded(&self, url: &str) -> Result<TransportResponse> {
        let response = self
            .transport
            .get(url, self.limits.max_response_bytes)
            .await?;
        if !(200..300).contains(&response.status) {
            return Err(upstream_status(response.status));
        }
        if !response
            .header("content-type")
            .is_some_and(media_type_is_json)
        {
            return Err(anyhow!(DomainError::upstream_error(
                "git_response_not_json"
            )));
        }
        Ok(response)
    }

    fn parse_json<U: serde::de::DeserializeOwned>(
        &self,
        response: &TransportResponse,
    ) -> Result<U> {
        serde_json::from_slice(&response.body)
            .map_err(|_| anyhow!(DomainError::upstream_error("git_response_malformed")))
    }

    fn accept_entry(
        &self,
        entry: super::forgejo_schemas::GitEntry,
        files: &mut Vec<TreeFileEntry>,
        excluded_files: &mut usize,
        too_large_files: &mut usize,
    ) -> Result<()> {
        if entry.kind != TREE_ENTRY_BLOB_TYPE {
            return Ok(());
        }
        if entry.mode.as_deref() == Some(SYMLINK_MODE) {
            *excluded_files += 1;
            return Ok(());
        }
        let declared_size = entry
            .size
            .ok_or_else(|| anyhow!(DomainError::upstream_error("git_tree_entry_missing_size")))?;
        if check_declared_length(Some(declared_size), self.limits.blob_budget()).is_err() {
            *excluded_files += 1;
            *too_large_files += 1;
            return Ok(());
        }
        let Ok(path) = SafeTreePath::parse(&entry.path) else {
            *excluded_files += 1;
            return Ok(());
        };
        files.push(TreeFileEntry {
            path,
            blob_sha: SafeSha::parse(&entry.sha)?,
            size: Some(declared_size),
        });
        Ok(())
    }
}

fn select_reference(references: Vec<Reference>, safe_ref: &SafeRef) -> Result<Reference> {
    let api_path = safe_ref.api_path();
    let with_prefix = format!("refs/{api_path}");
    let mut matching = references.into_iter().filter(|reference| {
        reference.ref_name == safe_ref.as_str()
            || reference.ref_name == api_path
            || reference.ref_name == with_prefix
    });
    let selected = matching
        .next()
        .ok_or_else(|| anyhow!(DomainError::upstream_error("git_ref_not_found")))?;
    if matching.next().is_some() {
        return Err(anyhow!(DomainError::upstream_error("git_ref_ambiguous")));
    }
    Ok(selected)
}

#[async_trait]
impl<T: GitHttpTransport + ?Sized + Sync> RepositoryAcquirer for ForgejoAcquirer<T> {
    async fn resolve_ref(&self, safe_ref: &SafeRef) -> Result<SafeSha> {
        self.resolve_ref_inner(safe_ref).await
    }

    async fn list_tree(&self, commit_sha: &SafeSha) -> Result<TreeListing> {
        let mut files = Vec::new();
        let mut excluded_files = 0usize;
        let mut too_large_files = 0usize;
        let mut page = 1u32;
        let mut page_budget = MAX_TREE_PAGES;
        let mut tree_sha: Option<SafeSha> = None;
        let mut declared_total: Option<usize> = None;
        let mut seen_paths: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        loop {
            let url = forgejo_tree_endpoint(
                &self.origin,
                &self.coordinates,
                commit_sha.as_str(),
                page,
                TREE_PAGE_SIZE,
            );
            let response = self.get_bounded(&url).await?;
            let parsed: GitTreeResponse = self.parse_json(&response)?;
            // Forgejo reports the tree object's SHA here, not the requested
            // commit SHA. Every page must describe the same tree object; the
            // pinned commit remains the identity the listing is recorded under.
            let page_tree_sha = SafeSha::parse(&parsed.sha)?;
            match &tree_sha {
                Some(expected) if expected.as_str() != page_tree_sha.as_str() => {
                    return Err(anyhow!(DomainError::upstream_error(
                        "git_tree_sha_mismatch"
                    )));
                }
                Some(_) => {}
                None => tree_sha = Some(page_tree_sha),
            }
            if parsed.page != 0 && parsed.page != i64::from(page) {
                return Err(anyhow!(DomainError::upstream_error(
                    "git_tree_page_mismatch"
                )));
            }
            let page_total = usize::try_from(parsed.total_count)
                .map_err(|_| anyhow!(DomainError::upstream_error("git_tree_total_invalid")))?;
            if page_total > self.limits.max_tree_entries {
                return Err(DomainError::payload_too_large("git_tree_too_large").into());
            }
            let declared = match declared_total {
                Some(expected) if expected != page_total => {
                    return Err(anyhow!(DomainError::upstream_error(
                        "git_tree_total_mismatch"
                    )));
                }
                Some(expected) => expected,
                None => {
                    declared_total = Some(page_total);
                    page_total
                }
            };
            // Forgejo may enforce a smaller page cap than the requested size.
            // The observed page length is therefore the bound for how many
            // pages complete coverage can need, never the requested size.
            let page_len = parsed.tree.len();
            if page == 1 && declared > 0 && page_len > 0 {
                let required_pages = declared.div_ceil(page_len).saturating_add(1);
                page_budget = u32::try_from(required_pages)
                    .unwrap_or(u32::MAX)
                    .min(MAX_TREE_PAGES)
                    .max(1);
            }
            let mut new_paths = 0usize;
            for entry in parsed.tree {
                if let Some(previous_sha) = seen_paths.get(&entry.path) {
                    if previous_sha != &entry.sha {
                        return Err(anyhow!(DomainError::upstream_error(
                            "git_tree_duplicate_entry"
                        )));
                    }
                    continue;
                }
                seen_paths.insert(entry.path.clone(), entry.sha.clone());
                new_paths += 1;
                self.accept_entry(entry, &mut files, &mut excluded_files, &mut too_large_files)?;
            }
            let distinct_entries = seen_paths.len();
            if distinct_entries > declared {
                return Err(anyhow!(DomainError::upstream_error(
                    "git_tree_total_mismatch"
                )));
            }
            // Completion is distinct immutable-tree coverage, not the
            // provider's final-page `truncated` flag: Forgejo v17 keeps it true
            // on a nonempty final page whenever the tree exceeds its page cap.
            if distinct_entries == declared {
                break;
            }
            if !parsed.truncated {
                break;
            }
            if page_len == 0 || new_paths == 0 {
                return Err(anyhow!(DomainError::upstream_error("git_tree_incomplete")));
            }
            if page >= page_budget {
                return Err(anyhow!(DomainError::upstream_error("git_tree_truncated")));
            }
            page += 1;
        }
        if declared_total != Some(seen_paths.len()) {
            return Err(anyhow!(DomainError::upstream_error("git_tree_incomplete")));
        }
        Ok(TreeListing {
            #[cfg(test)]
            commit_sha: commit_sha.clone(),
            files,
            excluded_files,
            too_large_files,
        })
    }

    async fn fetch_blob(&self, blob_sha: &SafeSha) -> Result<BlobContent> {
        let url = forgejo_blob_endpoint(&self.origin, &self.coordinates, blob_sha.as_str());
        let response = self.get_bounded(&url).await?;
        let parsed: GitBlobResponse = self.parse_json(&response)?;
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
        let decoded = self.decode_blob_content(&parsed.content)?;
        if decoded.len() as u64 != declared_size {
            return Err(anyhow!(DomainError::upstream_error(
                "git_blob_size_mismatch"
            )));
        }
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
            #[cfg(test)]
            declared_size,
            bytes: Bytes::from(decoded),
        })
    }
}

fn upstream_status(status: u16) -> anyhow::Error {
    anyhow!(DomainError::upstream_error(format!(
        "git_http_status_{status}"
    )))
}

fn media_type_is_json(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}

#[cfg(test)]
#[path = "forgejo_client_tests.rs"]
mod tests;
