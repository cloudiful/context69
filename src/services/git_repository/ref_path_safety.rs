//! Ref and path safety for bounded public Git acquisition (issue #681
//! phase 3A). Everything that flows into a request path or a metadata
//! response is parsed into a validated type first: refs cannot traverse or
//! inject separators, tree paths are normalized relative segments, and SHA
//! values are fixed-length lowercase hex.

use anyhow::{Result, anyhow};

use crate::domain_errors::DomainError;

/// Maximum length of a Git ref name accepted for acquisition.
pub(crate) const MAX_REF_LENGTH: usize = 200;

/// A validated ref spec: either a full ref (`refs/heads/main`, `refs/tags/v1`)
/// or the bare `HEAD` alias.
///
/// Both the `refs/...` prefix and `HEAD` are the caller-facing forms; endpoint
/// construction strips `refs/` because GitHub's `git/ref/{ref}` endpoint
/// expects the short form (`heads/main`). The raw value is safe to embed in a
/// URL path segment: the allowed charset excludes separators-in-segment,
/// `..`, whitespace, control characters, and the `@` reflog introducer.
/// Parsing is strict — no trimming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeRef(String);

impl SafeRef {
    /// Parse a ref strictly: no trimming, so any surrounding whitespace is
    /// rejected rather than silently accepted.
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let value = raw;
        if value.is_empty() || value.len() > MAX_REF_LENGTH {
            return Err(invalid_ref());
        }
        // Structural boundaries first: refs must be `refs/...` unless they
        // are the singular `HEAD`, so `--upload-pack`, option-like strings,
        // and host-side paths cannot masquerade as refs.
        if value != "HEAD" && !value.starts_with("refs/") {
            return Err(invalid_ref());
        }
        if value.ends_with('/') || value.contains("//") || value.contains("..") {
            return Err(invalid_ref());
        }
        for segment in value.split('/') {
            validate_segment(segment)?;
        }
        Ok(Self(value.to_owned()))
    }

    /// Validated raw ref as supplied (`refs/heads/main`, `HEAD`).
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Endpoint path form for GitHub's `git/ref/{ref}` API: the full ref
    /// minus the `refs/` prefix (`heads/main`), or `HEAD` unchanged. The
    /// provider 404s on the `refs/`-prefixed form.
    pub(crate) fn api_path(&self) -> &str {
        self.0.strip_prefix("refs/").unwrap_or(&self.0)
    }
}

fn validate_segment(segment: &str) -> Result<()> {
    // `@` is rejected outright so the `@{` reflog introducer cannot appear;
    // `:` stays allowed (it is legal in refnames but never in our endpoint
    // position after percent-encoding).
    let valid = !segment.is_empty()
        && segment != "."
        && segment != "@"
        && !segment.contains('@')
        && segment.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '+' | ':')
        })
        && !segment.starts_with('.')
        && !segment.ends_with(".lock");
    if valid { Ok(()) } else { Err(invalid_ref()) }
}

fn invalid_ref() -> anyhow::Error {
    anyhow!(DomainError::invalid_argument("git_ref_invalid"))
}

/// Maximum length of one tree path accepted for acquisition.
pub(crate) const MAX_TREE_PATH_LENGTH: usize = 512;

/// Maximum depth (segment count) of a tree path accepted for acquisition.
pub(crate) const MAX_TREE_PATH_DEPTH: usize = 32;

/// Why a tree path was not accepted for acquisition.
///
/// Provider paths are never fed to the endpoint builders, so a rejected
/// path only excludes one entry from a listing; it is never a request
/// hazard. The reason is kept for per-entry exclusion decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreePathRejection {
    /// Empty, oversized, or too deep for the acquisition bounds.
    OutOfBounds,
    /// Traversal or separator hazard: `.`, `..`, empty segment, backslash,
    /// or NUL.
    Traversal,
    /// Control character anywhere in the path.
    Control,
    /// Path escapes the repository root (absolute).
    Absolute,
}

/// Classify a provider-reported tree path.
///
/// Real-world repository names are accepted liberally (leading-dot
/// directories like `.github`, lockfiles like `Cargo.lock`, scoped package
/// names with `@`, spaces, Unicode): the endpoint builders never embed tree
/// paths, so the only hazards are traversal structure and control bytes.
pub(crate) fn classify_tree_path(raw: &str) -> Result<(), TreePathRejection> {
    if raw.is_empty() || raw.len() > MAX_TREE_PATH_LENGTH {
        return Err(TreePathRejection::OutOfBounds);
    }
    if raw.starts_with('/') {
        return Err(TreePathRejection::Absolute);
    }
    if raw.chars().any(char::is_control) {
        return Err(TreePathRejection::Control);
    }
    let segments: Vec<&str> = raw.split('/').collect();
    if segments.len() > MAX_TREE_PATH_DEPTH {
        return Err(TreePathRejection::OutOfBounds);
    }
    for segment in &segments {
        if segment.is_empty() || *segment == "." || *segment == ".." {
            return Err(TreePathRejection::Traversal);
        }
        if segment.contains('\\') {
            return Err(TreePathRejection::Traversal);
        }
    }
    Ok(())
}

/// A validated relative tree path (`README.md`, `.github/workflows/ci.yml`).
///
/// Wraps a path that passed [`classify_tree_path`]; construction is
/// restricted to this module so the classification cannot be bypassed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeTreePath(String);

impl SafeTreePath {
    /// Validate and accept a provider-reported tree path.
    pub(crate) fn parse(raw: &str) -> Result<Self, TreePathRejection> {
        classify_tree_path(raw)?;
        Ok(Self(raw.to_owned()))
    }

    /// Validated repository-relative path.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Maximum length of an accepted object SHA.
pub(crate) const SHA_MAX_LENGTH: usize = 64;

/// A validated lowercase hex object SHA (`tree`, `blob`, or commit id).
///
/// Only lowercase hex is accepted, so SHA values returned by provider
/// responses cannot smuggle path structure into subsequent endpoint
/// requests. Length must be either 40 (SHA-1) or 64 (SHA-256).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeSha(String);

impl SafeSha {
    /// Parse a provider-reported SHA strictly: no trimming, lowercase hex,
    /// exactly 40 (SHA-1) or 64 (SHA-256) characters. A failure means the
    /// provider sent a malformed identifier, so it classifies as upstream.
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let valid = matches!(raw.len(), 40 | 64)
            && raw.bytes().all(|byte| byte.is_ascii_hexdigit())
            && !raw.bytes().any(|byte| byte.is_ascii_uppercase());
        if valid {
            Ok(Self(raw.to_owned()))
        } else {
            Err(anyhow!(DomainError::upstream_error("git_sha_malformed")))
        }
    }

    /// Validated lowercase hex SHA, safe for endpoint construction.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "ref_path_safety_tests.rs"]
mod tests;
