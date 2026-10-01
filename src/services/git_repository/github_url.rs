use anyhow::Result;
use reqwest::Url;

use super::ref_path_safety::SafeRef;
use crate::domain_errors::DomainError;

/// Canonical GitHub repository locator.
///
/// `owner` and `name` are validated identifiers, not raw URL path
/// fragments, so they are safe to embed in API endpoint paths without
/// further escaping; [`repo_path`] still percent-encodes as
/// defense-in-depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitHubRepoCoordinates {
    pub owner: String,
    pub name: String,
}

/// Parse a canonical public GitHub HTTPS URL into repository coordinates.
///
/// Accepts exactly one shape: `https://github.com/{owner}/{name}` with no
/// credentials, no non-default port, no query string, and no fragment. A
/// trailing slash is allowed; extra path segments (`/tree/...`, `/blob/...`,
/// `/archive/...`) are rejected so callers cannot smuggle endpoint structure
/// through the URL. A single `.git` suffix on the repository name is
/// stripped after validation.
pub(crate) fn parse_canonical_github_url(canonical_url: &str) -> Result<GitHubRepoCoordinates> {
    let invalid = || DomainError::invalid_argument("invalid_git_repository_url");
    let url = Url::parse(canonical_url.trim()).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid().into());
    }
    let mut segments = url
        .path_segments()
        .ok_or_else(invalid)?
        .filter(|segment| !segment.is_empty());
    let owner = segments.next().ok_or_else(invalid)?;
    let name = segments.next().ok_or_else(invalid)?;
    if segments.next().is_some() {
        return Err(invalid().into());
    }
    validate_owner(owner)?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    validate_repo_name(name)?;
    Ok(GitHubRepoCoordinates {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// GitHub owner (user or organization) login charset.
fn validate_owner(owner: &str) -> Result<()> {
    let valid = !owner.is_empty()
        && owner.len() <= 100
        && owner != "."
        && owner != ".."
        && owner
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if valid {
        Ok(())
    } else {
        Err(DomainError::invalid_argument("invalid_git_repository_url").into())
    }
}

/// Repository names additionally allow `_` and `.`.
fn validate_repo_name(name: &str) -> Result<()> {
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

/// Percent-encoded `{owner}/{repo}` request path segment.
pub(crate) fn repo_path(coordinates: &GitHubRepoCoordinates) -> String {
    format!(
        "{}/{}",
        encode_segment(&coordinates.owner),
        encode_segment(&coordinates.name)
    )
}

/// Percent-encode one already-validated path segment.
///
/// Inputs are validated before reaching this helper (`[A-Za-z0-9-._]` for
/// repository segments, `SafeRef`/`SafeTreePath` charset elsewhere), so the
/// encoding is a defense-in-depth identity for approved input, but it is a
/// real percent-encoder: any byte outside the unreserved set is escaped.
fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Build the refs endpoint URL for one ref (short `api_path` form).
pub(crate) fn ref_endpoint(coordinates: &GitHubRepoCoordinates, safe_ref: &SafeRef) -> Url {
    endpoint(&format!(
        "repos/{}/git/ref/{}",
        repo_path(coordinates),
        encode_segment(safe_ref.api_path())
    ))
}

/// Build the repository metadata endpoint, which reports `default_branch`.
pub(crate) fn repo_metadata_endpoint(coordinates: &GitHubRepoCoordinates) -> Url {
    endpoint(&format!("repos/{}", repo_path(coordinates)))
}

/// Build the annotated-tag endpoint (`git/tags/{sha}`), which reports the
/// tag's target object and kind.
pub(crate) fn tag_endpoint(coordinates: &GitHubRepoCoordinates, sha: &str) -> Url {
    endpoint(&format!(
        "repos/{}/git/tags/{}",
        repo_path(coordinates),
        encode_segment(sha)
    ))
}

/// Build the recursive tree endpoint for a commit (or other) id. The
/// provider resolves it to the commit's tree and echoes the same id.
pub(crate) fn tree_endpoint(coordinates: &GitHubRepoCoordinates, commit_sha: &str) -> Url {
    endpoint(&format!(
        "repos/{}/git/trees/{commit_sha}?recursive=1",
        repo_path(coordinates)
    ))
}

/// Build the blob endpoint URL for one blob SHA.
pub(crate) fn blob_endpoint(coordinates: &GitHubRepoCoordinates, blob_sha: &str) -> Url {
    endpoint(&format!(
        "repos/{}/git/blobs/{blob_sha}",
        repo_path(coordinates)
    ))
}

fn endpoint(path_and_query: &str) -> Url {
    // `github.com` plus a path built from validated components always parses.
    // `expect` cannot fire: scheme/host are constants and the path parts are
    // validated or percent-encoded, so no parse failure is reachable.
    match Url::parse(&format!("https://api.github.com/{path_and_query}")) {
        Ok(url) => url,
        Err(error) => {
            if cfg!(debug_assertions) {
                panic!("validated github endpoint failed to parse: {error}");
            }
            Url::parse("https://api.github.com/").expect("constant github origin parses")
        }
    }
}

#[cfg(test)]
#[path = "github_url_tests.rs"]
mod tests;
