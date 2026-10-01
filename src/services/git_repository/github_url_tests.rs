use super::super::github_url::{
    blob_endpoint, parse_canonical_github_url, ref_endpoint, repo_metadata_endpoint, repo_path,
    tag_endpoint, tree_endpoint,
};
use super::super::ref_path_safety::{SafeRef, SafeSha};

#[test]
fn parses_canonical_owner_and_name() {
    let parsed = parse_canonical_github_url("https://github.com/octocat/Hello-World").unwrap();
    assert_eq!(parsed.owner, "octocat");
    assert_eq!(parsed.name, "Hello-World");
}

#[test]
fn accepts_single_trailing_slash_and_strips_git_suffix() {
    let trailing = parse_canonical_github_url("https://github.com/octo/repo/").unwrap();
    assert_eq!(
        (trailing.owner.as_str(), trailing.name.as_str()),
        ("octo", "repo")
    );
    let git = parse_canonical_github_url("https://github.com/octo/repo.git").unwrap();
    assert_eq!(git.name, "repo");
}

#[test]
fn rejects_non_https_schemes_and_lookalike_hosts() {
    for url in [
        "http://github.com/octo/repo",
        "https://api.github.com/octo/repo",
        "https://github.com.evil.test/octo/repo",
        "https://raw.githubusercontent.com/octo/repo/main/x",
    ] {
        assert!(
            parse_canonical_github_url(url).is_err(),
            "must reject {url}"
        );
    }
}

#[test]
fn rejects_credentials_ports_query_fragment_and_extra_segments() {
    for url in [
        "https://user:pass@github.com/octo/repo",
        "https://github.com:8443/octo/repo",
        "https://github.com/octo/repo?ref=main",
        "https://github.com/octo/repo#readme",
        "https://github.com/octo/repo/tree/main",
        "https://github.com/octo/repo/blob/main/README.md",
        "https://github.com/octo",
        "https://github.com/",
    ] {
        assert!(
            parse_canonical_github_url(url).is_err(),
            "must reject {url}"
        );
    }
}

#[test]
fn rejects_invalid_owner_or_name_characters() {
    for url in [
        "https://github.com/../repo",
        "https://github.com/octo/../repo",
        "https://github.com/octi on/repo",
        "https://github.com/octo/re po",
        "https://github.com/octo/repo.git/x",
    ] {
        assert!(
            parse_canonical_github_url(url).is_err(),
            "must reject {url}"
        );
    }
}

#[test]
fn repo_path_encodes_segments_stably() {
    let parsed = parse_canonical_github_url("https://github.com/Octo-1/Hello_World").unwrap();
    assert_eq!(repo_path(&parsed), "Octo-1/Hello_World");
}

#[test]
fn ref_endpoint_uses_the_short_api_path_form() {
    let parsed = parse_canonical_github_url("https://github.com/octo/repo").unwrap();
    let reference = SafeRef::parse("refs/heads/main").unwrap();
    let url = ref_endpoint(&parsed, &reference).to_string();
    // GitHub's `git/ref/{ref}` endpoint 404s on the `refs/`-prefixed form;
    // only the stripped form is a valid request.
    assert_eq!(
        url,
        "https://api.github.com/repos/octo/repo/git/ref/heads%2Fmain"
    );
    let tag = SafeRef::parse("refs/tags/v1.0.0").unwrap();
    assert_eq!(
        ref_endpoint(&parsed, &tag).to_string(),
        "https://api.github.com/repos/octo/repo/git/ref/tags%2Fv1.0.0"
    );
    let head = SafeRef::parse("HEAD").unwrap();
    assert_eq!(
        ref_endpoint(&parsed, &head).to_string(),
        "https://api.github.com/repos/octo/repo/git/ref/HEAD"
    );
}

#[test]
fn repo_metadata_endpoint_reports_default_branch_metadata() {
    let parsed = parse_canonical_github_url("https://github.com/octo/repo").unwrap();
    assert_eq!(
        repo_metadata_endpoint(&parsed).to_string(),
        "https://api.github.com/repos/octo/repo"
    );
}

#[test]
fn tag_endpoint_targets_the_git_tags_namespace() {
    let parsed = parse_canonical_github_url("https://github.com/octo/repo").unwrap();
    let sha = SafeSha::parse(&"b".repeat(40)).unwrap();
    assert_eq!(
        tag_endpoint(&parsed, sha.as_str()).to_string(),
        format!(
            "https://api.github.com/repos/octo/repo/git/tags/{}",
            "b".repeat(40)
        )
    );
}

#[test]
fn tree_endpoint_carries_the_requested_commit_identity() {
    // The provider resolves `git/trees/{id}` for a commit id and echoes the
    // same id back; the endpoint takes that commit identity verbatim.
    let parsed = parse_canonical_github_url("https://github.com/octo/repo").unwrap();
    let commit = SafeSha::parse(&"a".repeat(40)).unwrap();
    assert_eq!(
        tree_endpoint(&parsed, commit.as_str()).to_string(),
        format!(
            "https://api.github.com/repos/octo/repo/git/trees/{}?recursive=1",
            "a".repeat(40)
        )
    );
}

#[test]
fn endpoints_are_stable_against_the_approved_host() {
    let parsed = parse_canonical_github_url("https://github.com/octo/repo").unwrap();
    let sha = SafeSha::parse(&"a".repeat(40)).unwrap();

    let tree_url = tree_endpoint(&parsed, sha.as_str()).to_string();
    assert_eq!(
        tree_url,
        format!(
            "https://api.github.com/repos/octo/repo/git/trees/{}?recursive=1",
            "a".repeat(40)
        )
    );

    let blob_url = blob_endpoint(&parsed, sha.as_str()).to_string();
    assert_eq!(
        blob_url,
        format!(
            "https://api.github.com/repos/octo/repo/git/blobs/{}",
            "a".repeat(40)
        )
    );
    for url in [tree_url, blob_url] {
        assert!(url.starts_with("https://api.github.com/"), "{url}");
    }
}
