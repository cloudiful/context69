//! Canonical provider URLs and exact connection-origin binding.
//!
//! GitHub repository URLs are fixed to `https://github.com/{owner}/{name}`.
//! Forgejo instances may live under a webroot, so a Forgejo repository URL is
//! only canonical against the connection that established the instance origin:
//! `https://{host}{webroot}/{owner}/{name}`. The connection's stored base URL is
//! an API base URL (`https://{host}{webroot}` or `.../api/v1`), so the same
//! webroot is derived from it and every repository URL is matched against it
//! before any endpoint is built.

use anyhow::Result;
use reqwest::Url;

use crate::contracts::sources::GitProviderKind;
use crate::domain_errors::DomainError;

/// A validated Forgejo instance identity: scheme, host, optional port, and
/// webroot prefix. The webroot never ends in `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForgejoOrigin {
    host: String,
    port: Option<u16>,
    webroot: String,
}

/// Validated Forgejo repository path coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForgejoRepoCoordinates {
    pub owner: String,
    pub name: String,
}

impl ForgejoOrigin {
    /// Parse a connection base URL into the instance identity.
    ///
    /// Only HTTPS without userinfo, query, or fragment is accepted; a trailing
    /// `/api/v1` is stripped so the connection may be stored either as the
    /// instance root or as its API base. Path traversal is rejected.
    pub(crate) fn from_base_url(base_url: &str) -> Result<Self> {
        let invalid = || DomainError::invalid_argument("git_connection_origin_invalid");
        let url = Url::parse(base_url.trim()).map_err(|_| invalid())?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid().into());
        }
        let host = url.host_str().ok_or_else(invalid)?.to_owned();
        if host.is_empty() {
            return Err(invalid().into());
        }
        let mut webroot = url.path().trim_end_matches('/').to_owned();
        if webroot.ends_with("/api/v1") {
            webroot.truncate(webroot.len() - "/api/v1".len());
            webroot = webroot.trim_end_matches('/').to_owned();
        }
        if !webroot.is_empty() && !safe_webroot(&webroot) {
            return Err(invalid().into());
        }
        Ok(Self {
            host,
            port: url.port(),
            webroot,
        })
    }

    /// Parse a repository URL and require it to sit exactly on this origin.
    pub(crate) fn parse_repository_url(
        &self,
        canonical_url: &str,
    ) -> Result<ForgejoRepoCoordinates> {
        let invalid = || DomainError::invalid_argument("invalid_git_repository_url");
        let url = Url::parse(canonical_url.trim()).map_err(|_| invalid())?;
        if url.scheme() != "https"
            || url.host_str() != Some(self.host.as_str())
            || url.port() != self.port
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid().into());
        }
        let path = url.path().trim_end_matches('/');
        let prefix = self.webroot.as_str();
        let remainder = if prefix.is_empty() {
            path
        } else {
            let remainder = path.strip_prefix(prefix).ok_or_else(invalid)?;
            if remainder.is_empty() {
                return Err(invalid().into());
            }
            remainder.strip_prefix('/').ok_or_else(invalid)?
        };
        let remainder = remainder.strip_prefix('/').unwrap_or(remainder);
        let mut segments = remainder.split('/').filter(|segment| !segment.is_empty());
        let owner = segments.next().ok_or_else(invalid)?;
        let name = segments.next().ok_or_else(invalid)?;
        if segments.next().is_some() {
            return Err(invalid().into());
        }
        validate_forgejo_owner(owner)?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        validate_forgejo_name(name)?;
        Ok(ForgejoRepoCoordinates {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// The canonical repository URL for coordinates on this origin.
    pub(crate) fn canonical_repository_url(&self, coordinates: &ForgejoRepoCoordinates) -> String {
        let authority = match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        };
        format!(
            "https://{authority}{}/{}/{}",
            self.webroot, coordinates.owner, coordinates.name
        )
    }

    /// The Forgejo REST API base (`https://{host}{webroot}/api/v1`).
    pub(crate) fn api_base(&self) -> String {
        format!("https://{}{}/api/v1", self.authority(), self.webroot)
    }

    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        }
    }
}

fn safe_webroot(webroot: &str) -> bool {
    webroot.starts_with('/')
        && !webroot.contains("//")
        && !webroot.contains("..")
        && !webroot.ends_with('/')
        && webroot
            .split('/')
            .skip(1)
            .all(|segment| !segment.is_empty())
}

fn validate_forgejo_owner(owner: &str) -> Result<()> {
    let valid = !owner.is_empty()
        && owner.len() <= 100
        && owner != "."
        && owner != ".."
        && owner.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(DomainError::invalid_argument("invalid_git_repository_url").into())
    }
}

fn validate_forgejo_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(DomainError::invalid_argument("invalid_git_repository_url").into())
    }
}

/// Whether a GitHub connection base URL names the approved GitHub API origin.
pub(crate) fn github_connection_origin_matches(base_url: &str) -> bool {
    let Ok(url) = Url::parse(base_url.trim()) else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("api.github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    matches!(url.path().trim_end_matches('/'), "" | "/api/v3")
}

/// Validate that a connection can acquire the given provider's repository URL.
///
/// GitHub is checked against its fixed API origin; Forgejo must share the
/// canonical repository origin derived from the connection base URL.
pub(crate) fn validate_connection_origin(
    provider: GitProviderKind,
    base_url: &str,
    canonical_url: &str,
) -> Result<()> {
    match provider {
        GitProviderKind::GitHub => {
            super::github_url::parse_canonical_github_url(canonical_url)?;
            if github_connection_origin_matches(base_url) {
                Ok(())
            } else {
                Err(DomainError::conflict("git_connection_origin_mismatch").into())
            }
        }
        GitProviderKind::Forgejo => {
            let origin = ForgejoOrigin::from_base_url(base_url).map_err(|_| {
                anyhow::anyhow!(DomainError::conflict("git_connection_origin_mismatch"))
            })?;
            origin.parse_repository_url(canonical_url).map_err(|_| {
                anyhow::anyhow!(DomainError::conflict("git_connection_origin_mismatch"))
            })?;
            Ok(())
        }
        _ => Err(DomainError::invalid_argument("git_provider_unsupported").into()),
    }
}

fn forgejo_repo_path(coordinates: &ForgejoRepoCoordinates) -> String {
    format!(
        "{}/{}",
        super::github_url::encode_segment(&coordinates.owner),
        super::github_url::encode_segment(&coordinates.name)
    )
}

pub(crate) fn forgejo_repo_endpoint(
    origin: &ForgejoOrigin,
    coordinates: &ForgejoRepoCoordinates,
) -> String {
    format!(
        "{}/repos/{}",
        origin.api_base(),
        forgejo_repo_path(coordinates)
    )
}

pub(crate) fn forgejo_refs_endpoint(
    origin: &ForgejoOrigin,
    coordinates: &ForgejoRepoCoordinates,
    safe_ref: &super::ref_path_safety::SafeRef,
) -> String {
    // `SafeRef` permits only validated ref segments and `/` separators, so the
    // short form is already path-safe. It stays unencoded because Forgejo's
    // refs route matches the slash-separated short ref rather than one segment.
    format!(
        "{}/git/refs/{}",
        forgejo_repo_endpoint(origin, coordinates),
        safe_ref.api_path()
    )
}

pub(crate) fn forgejo_tree_endpoint(
    origin: &ForgejoOrigin,
    coordinates: &ForgejoRepoCoordinates,
    commit_sha: &str,
    page: u32,
    per_page: u32,
) -> String {
    format!(
        "{}/git/trees/{}?recursive=true&page={page}&per_page={per_page}",
        forgejo_repo_endpoint(origin, coordinates),
        super::github_url::encode_segment(commit_sha)
    )
}

pub(crate) fn forgejo_blob_endpoint(
    origin: &ForgejoOrigin,
    coordinates: &ForgejoRepoCoordinates,
    blob_sha: &str,
) -> String {
    format!(
        "{}/git/blobs/{}",
        forgejo_repo_endpoint(origin, coordinates),
        super::github_url::encode_segment(blob_sha)
    )
}

pub(crate) fn forgejo_tag_endpoint(
    origin: &ForgejoOrigin,
    coordinates: &ForgejoRepoCoordinates,
    tag_sha: &str,
) -> String {
    format!(
        "{}/git/tags/{}",
        forgejo_repo_endpoint(origin, coordinates),
        super::github_url::encode_segment(tag_sha)
    )
}

#[cfg(test)]
#[path = "provider_url_tests.rs"]
mod tests;
