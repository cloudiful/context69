//! Ref-resolution helpers for the bounded public GitHub client (issue #681
//! phase 3A): the resolution flows live on the client; this module carries
//! the bounded-dereference policy and metadata validation it applies.

use anyhow::{Result, anyhow};

use crate::domain_errors::DomainError;

use super::ref_path_safety::SafeRef;

/// How deep a tag→tag→…→commit chain may be followed before the client
/// refuses. Git itself allows deeper chains, but acquisition only needs the
/// commit a ref points at, and a hard depth keeps a hostile repository from
/// turning one resolution into an unbounded request walk.
pub(crate) const MAX_TAG_DEREF_DEPTH: usize = 4;

/// Validate a provider-reported `default_branch` before it becomes a ref
/// request path.
///
/// The metadata value is a branch name, not a full ref; the API path form of
/// a branch is `heads/<name>`, which [`SafeRef`] accepts only with the
/// `refs/` prefix, so re-assemble and validate structurally. A missing or
/// malformed repository record is upstream data and classifies accordingly.
pub(crate) fn validated_default_branch(default_branch: &str) -> Result<SafeRef> {
    if default_branch.trim().is_empty() {
        return Err(anyhow!(DomainError::upstream_error(
            "git_default_branch_missing"
        )));
    }
    SafeRef::parse(&format!("refs/heads/{default_branch}"))
}
