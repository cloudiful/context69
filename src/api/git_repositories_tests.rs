//! Focused handler/validation tests for the group-scoped Git repository routes
//! (issue #681 work unit 3C2).
//!
//! These exercise the pure request validation that runs before any database or
//! provider work: canonical GitHub URL derivation, ref/branch/pin safety, and
//! the `Idempotency-Key` reader. Route authorization and persistence are left to
//! the group-access and persistence suites.

use axum::http::{HeaderMap, HeaderValue};

use crate::contracts::sources::{
    GitIndexProfile, GitRefreshPolicy, GitRepositoryRegistrationRequest,
};

use super::{idempotency_key, validated_source};

fn request(
    canonical_url: &str,
    target_ref: &str,
    pinned_commit: Option<&str>,
) -> GitRepositoryRegistrationRequest {
    GitRepositoryRegistrationRequest {
        canonical_url: canonical_url.to_string(),
        default_branch: "main".to_string(),
        target_ref: target_ref.to_string(),
        pinned_commit: pinned_commit.map(str::to_string),
        index_profile: GitIndexProfile::Lexical,
        refresh_policy: GitRefreshPolicy::Manual,
    }
}

fn error_message(error: anyhow::Error) -> String {
    error.to_string()
}

#[test]
fn canonical_github_url_is_normalized_and_split() {
    let source = validated_source(&request(
        "https://github.com/Cloudiful/Context69.git/",
        "refs/heads/main",
        None,
    ))
    .expect("valid registration");

    assert_eq!(
        source.provider,
        crate::contracts::sources::GitProviderKind::GitHub
    );
    assert_eq!(
        source.canonical_url,
        "https://github.com/Cloudiful/Context69"
    );
    assert_eq!(source.owner, "Cloudiful");
    assert_eq!(source.name, "Context69");
    assert_eq!(source.connection_key, None);
    assert_eq!(source.target_ref, "refs/heads/main");
    assert_eq!(source.target_commit_sha, None);
}

#[test]
fn registration_rejects_non_github_or_unsafe_urls() {
    for url in [
        "http://github.com/cloudiful/context69",
        "https://gitlab.com/cloudiful/context69",
        "https://github.com/cloudiful/context69/tree/main",
        "https://user:secret@github.com/cloudiful/context69",
        "https://github.com/cloudiful",
    ] {
        let error = validated_source(&request(url, "refs/heads/main", None))
            .expect_err("url must be rejected");
        assert_eq!(error_message(error), "invalid_git_repository_url");
    }
}

#[test]
fn registration_rejects_branch_ref_default_branch_mismatch() {
    let mut mismatched = request(
        "https://github.com/cloudiful/context69",
        "refs/heads/release",
        None,
    );
    mismatched.default_branch = "main".to_string();
    let error = validated_source(&mismatched).expect_err("branch mismatch must be rejected");
    assert_eq!(error_message(error), "git_ref_branch_mismatch");

    // A tag target is independent of the default branch and stays accepted.
    let tagged = request(
        "https://github.com/cloudiful/context69",
        "refs/tags/v1.2.3",
        None,
    );
    assert!(validated_source(&tagged).is_ok());
    // `HEAD` is accepted as the singular ref alias.
    assert!(
        validated_source(&request(
            "https://github.com/cloudiful/context69",
            "HEAD",
            None
        ))
        .is_ok()
    );
}

#[test]
fn registration_rejects_unsafe_refs_and_default_branches() {
    for target_ref in ["main", "refs/heads/", "refs/heads/..", "refs/heads/a b"] {
        let error = validated_source(&request(
            "https://github.com/cloudiful/context69",
            target_ref,
            None,
        ))
        .expect_err("unsafe ref must be rejected");
        assert_eq!(error_message(error), "git_ref_invalid");
    }

    let mut blank_branch = request("https://github.com/cloudiful/context69", "HEAD", None);
    blank_branch.default_branch = String::new();
    let error = validated_source(&blank_branch).expect_err("blank branch must be rejected");
    assert_eq!(error_message(error), "git_default_branch_invalid");
}

#[test]
fn registration_validates_optional_pinned_commit() {
    let valid = request(
        "https://github.com/cloudiful/context69",
        "refs/heads/main",
        Some("0123456789abcdef0123456789abcdef01234567"),
    );
    let source = validated_source(&valid).expect("lowercase 40-hex pin");
    assert_eq!(
        source.target_commit_sha.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );

    for pinned in ["", "not-a-sha", "0123456789ABCDEF0123456789ABCDEF01234567"] {
        let error = validated_source(&request(
            "https://github.com/cloudiful/context69",
            "refs/heads/main",
            Some(pinned),
        ))
        .expect_err("invalid pin must be rejected");
        assert_eq!(error_message(error), "git_commit_sha_invalid");
    }
}

#[test]
fn idempotency_key_reads_trimmed_header_only() {
    let mut headers = HeaderMap::new();
    assert_eq!(idempotency_key(&headers), None);

    headers.insert("idempotency-key", HeaderValue::from_static("   "));
    assert_eq!(idempotency_key(&headers), None);

    headers.insert("idempotency-key", HeaderValue::from_static("  ctx-key-1  "));
    assert_eq!(idempotency_key(&headers).as_deref(), Some("ctx-key-1"));
}
