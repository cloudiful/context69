//! Focused handler/validation tests for the group-scoped Git repository routes
//! (issue #681 work unit 3C2).
//!
//! These exercise the pure request validation that runs before any database or
//! provider work: canonical GitHub URL derivation, ref/branch/pin safety, and
//! the `Idempotency-Key` reader. Route authorization and persistence are left to
//! the group-access and persistence suites.

use axum::http::{HeaderMap, HeaderValue};

use crate::contracts::sources::{
    GitConnectionMode, GitIndexProfile, GitPinnedCommitPatch, GitProviderKind,
    GitRepositoryRegistrationRejection, GitRepositoryRegistrationRequest,
    GitRepositorySettingsUpdateRequest,
};

use super::{idempotency_key, validated_settings_update, validated_source};
use crate::contracts::Visibility;
use crate::db::{GitGroupOwnership, StoredGitProviderConnection};
use crate::domain_errors::DomainError;
use chrono::Utc;

fn request(
    canonical_url: &str,
    target_ref: &str,
    pinned_commit: Option<&str>,
) -> GitRepositoryRegistrationRequest {
    GitRepositoryRegistrationRequest {
        provider: GitProviderKind::GitHub,
        canonical_url: canonical_url.to_string(),
        default_branch: "main".to_string(),
        target_ref: target_ref.to_string(),
        pinned_commit: pinned_commit.map(str::to_string),
        connection_key: None,
        index_profile: GitIndexProfile::Lexical,
        settings: Default::default(),
    }
}

fn validated_public(
    request: &GitRepositoryRegistrationRequest,
) -> anyhow::Result<crate::db::NewGitRepositorySource> {
    validated_source(request, None, None)
}

fn stored_connection(
    provider: GitProviderKind,
    mode: GitConnectionMode,
    base_url: &str,
    credential: bool,
    disabled: bool,
) -> StoredGitProviderConnection {
    let now = Utc::now();
    StoredGitProviderConnection {
        group: GitGroupOwnership {
            group_id: 1,
            group_key: "research".to_string(),
            group_path: "research".to_string(),
            visibility: Visibility::Private,
        },
        connection_key: "forgejo-main".to_string(),
        provider,
        mode,
        display_name: "Forgejo".to_string(),
        base_url: base_url.to_string(),
        credential_secret_key: credential.then(|| "internal/secret/read".to_string()),
        webhook_secret_key: None,
        app_private_key_secret_key: None,
        disabled_at: disabled.then_some(now),
        created_at: now,
        updated_at: now,
    }
}

fn error_message(error: anyhow::Error) -> String {
    error.to_string()
}

#[test]
fn canonical_github_url_is_normalized_and_split() {
    let source = validated_public(&request(
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
    assert_eq!(source.pinned_commit_sha, None);
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
        let error = validated_public(&request(url, "refs/heads/main", None))
            .expect_err("url must be rejected");
        assert_eq!(error_message(error), "invalid_git_repository_url");
    }
}

#[test]
fn registration_accepts_a_branch_ref_outside_the_default_branch() {
    let mut release = request(
        "https://github.com/cloudiful/context69",
        "refs/heads/release",
        None,
    );
    release.default_branch = "main".to_string();
    let source =
        validated_public(&release).expect("a tracked branch is independent of the default");
    assert_eq!(source.target_ref, "refs/heads/release");
    assert_eq!(source.default_branch, "main");

    // A tag target is independent of the default branch and stays accepted.
    let tagged = request(
        "https://github.com/cloudiful/context69",
        "refs/tags/v1.2.3",
        None,
    );
    assert!(validated_public(&tagged).is_ok());
    // `HEAD` is accepted as the singular ref alias.
    assert!(
        validated_public(&request(
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
        let error = validated_public(&request(
            "https://github.com/cloudiful/context69",
            target_ref,
            None,
        ))
        .expect_err("unsafe ref must be rejected");
        assert_eq!(error_message(error), "git_ref_invalid");
    }

    let mut blank_branch = request("https://github.com/cloudiful/context69", "HEAD", None);
    blank_branch.default_branch = String::new();
    let error = validated_public(&blank_branch).expect_err("blank branch must be rejected");
    assert_eq!(error_message(error), "git_default_branch_invalid");
}

#[test]
fn registration_validates_optional_pinned_commit() {
    let valid = request(
        "https://github.com/cloudiful/context69",
        "refs/heads/main",
        Some("0123456789abcdef0123456789abcdef01234567"),
    );
    let source = validated_public(&valid).expect("lowercase 40-hex pin");
    assert_eq!(
        source.pinned_commit_sha.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );

    for pinned in ["", "not-a-sha", "0123456789ABCDEF0123456789ABCDEF01234567"] {
        let error = validated_public(&request(
            "https://github.com/cloudiful/context69",
            "refs/heads/main",
            Some(pinned),
        ))
        .expect_err("invalid pin must be rejected");
        assert_eq!(error_message(error), "git_commit_sha_invalid");
    }
}

#[test]
fn settings_update_normalizes_target_pin_and_profile() {
    let request = GitRepositorySettingsUpdateRequest {
        target_ref: Some("refs/tags/v2.0.0".to_string()),
        pinned_commit: GitPinnedCommitPatch::Set(
            "0123456789abcdef0123456789abcdef01234567".to_string(),
        ),
        index_profile: Some(GitIndexProfile::Hybrid),
        ..Default::default()
    };
    let validated = validated_settings_update(&request).expect("valid edits");
    assert_eq!(validated.target_ref.as_deref(), Some("refs/tags/v2.0.0"));
    assert_eq!(validated.pinned_commit_action, "set");
    assert_eq!(
        validated.pinned_commit_value.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(validated.index_profile, Some("hybrid"));

    let cleared = validated_settings_update(&GitRepositorySettingsUpdateRequest {
        pinned_commit: GitPinnedCommitPatch::Clear,
        ..Default::default()
    })
    .expect("clearing a pin needs no other edit");
    assert_eq!(cleared.pinned_commit_action, "clear");
    assert_eq!(cleared.pinned_commit_value, None);
    assert_eq!(cleared.target_ref, None);
    assert_eq!(cleared.index_profile, None);

    let kept = validated_settings_update(&GitRepositorySettingsUpdateRequest::default())
        .expect("an empty patch keeps every stored value");
    assert_eq!(kept.pinned_commit_action, "keep");
}

#[test]
fn settings_update_accepts_a_different_branch_and_rejects_unsafe_refs_and_pins() {
    let branch = GitRepositorySettingsUpdateRequest {
        target_ref: Some("refs/heads/release".to_string()),
        ..Default::default()
    };
    let validated =
        validated_settings_update(&branch).expect("a changed tracked branch is allowed");
    assert_eq!(validated.target_ref.as_deref(), Some("refs/heads/release"));

    let unsafe_ref = GitRepositorySettingsUpdateRequest {
        target_ref: Some("main".to_string()),
        ..Default::default()
    };
    assert_eq!(
        validated_settings_update(&unsafe_ref).expect_err("unsafe ref"),
        DomainError::invalid_argument("git_ref_invalid")
    );

    let uppercase = GitRepositorySettingsUpdateRequest {
        pinned_commit: GitPinnedCommitPatch::Set(
            "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
        ),
        ..Default::default()
    };
    assert_eq!(
        validated_settings_update(&uppercase).expect_err("uppercase pin"),
        DomainError::invalid_argument("git_commit_sha_invalid")
    );
}

fn forgejo_request(canonical_url: &str) -> GitRepositoryRegistrationRequest {
    GitRepositoryRegistrationRequest {
        provider: GitProviderKind::Forgejo,
        canonical_url: canonical_url.to_string(),
        default_branch: "main".to_string(),
        target_ref: "refs/heads/main".to_string(),
        pinned_commit: None,
        connection_key: Some("forgejo-main".to_string()),
        index_profile: GitIndexProfile::Lexical,
        settings: Default::default(),
    }
}

#[test]
fn forgejo_registration_requires_a_connection_origin() {
    let mut request = forgejo_request("https://forgejo.example/octo/repo");
    request.connection_key = None;
    let error = validated_source(&request, None, None).expect_err("connection is required");
    assert_eq!(
        error_message(error),
        GitRepositoryRegistrationRejection::ConnectionRequired.as_str()
    );
}

#[test]
fn unsupported_providers_are_rejected_at_registration() {
    for provider in [GitProviderKind::GitLab, GitProviderKind::Generic] {
        let mut request = request("https://gitlab.com/octo/repo", "HEAD", None);
        request.provider = provider;
        let error = validated_source(&request, None, None).expect_err("unsupported provider");
        assert_eq!(
            error_message(error),
            GitRepositoryRegistrationRejection::ProviderUnsupported.as_str()
        );
    }
}

#[test]
fn forgejo_registration_derives_the_canonical_url_on_the_connection_origin() {
    let connection = stored_connection(
        GitProviderKind::Forgejo,
        GitConnectionMode::Public,
        "https://forgejo.example/api/v1",
        false,
        false,
    );
    let source = validated_source(
        &forgejo_request("https://forgejo.example/octo/repo.git/"),
        Some("forgejo-main"),
        Some(&connection),
    )
    .expect("origin-bound registration");
    assert_eq!(source.provider, GitProviderKind::Forgejo);
    assert_eq!(source.canonical_url, "https://forgejo.example/octo/repo");
    assert_eq!(source.owner, "octo");
    assert_eq!(source.name, "repo");
    assert_eq!(source.connection_key.as_deref(), Some("forgejo-main"));
}

#[test]
fn registration_rejects_connection_origin_provider_mode_and_credential_mismatches() {
    let request = forgejo_request("https://forgejo.example/octo/repo");
    let mismatch = stored_connection(
        GitProviderKind::Forgejo,
        GitConnectionMode::Public,
        "https://evil.example",
        false,
        false,
    );
    assert_eq!(
        error_message(
            validated_source(&request, Some("forgejo-main"), Some(&mismatch))
                .expect_err("origin mismatch")
        ),
        GitRepositoryRegistrationRejection::ConnectionOriginMismatch.as_str()
    );
    let other_provider = stored_connection(
        GitProviderKind::GitHub,
        GitConnectionMode::Public,
        "https://api.github.com",
        false,
        false,
    );
    assert_eq!(
        error_message(
            validated_source(&request, Some("forgejo-main"), Some(&other_provider))
                .expect_err("provider mismatch")
        ),
        GitRepositoryRegistrationRejection::ConnectionProviderMismatch.as_str()
    );
    let disabled = stored_connection(
        GitProviderKind::Forgejo,
        GitConnectionMode::Public,
        "https://forgejo.example",
        false,
        true,
    );
    assert_eq!(
        error_message(
            validated_source(&request, Some("forgejo-main"), Some(&disabled))
                .expect_err("disabled")
        ),
        GitRepositoryRegistrationRejection::ConnectionDisabled.as_str()
    );
    let installation = stored_connection(
        GitProviderKind::Forgejo,
        GitConnectionMode::Installation,
        "https://forgejo.example",
        true,
        false,
    );
    assert_eq!(
        error_message(
            validated_source(&request, Some("forgejo-main"), Some(&installation))
                .expect_err("installation")
        ),
        GitRepositoryRegistrationRejection::ConnectionModeUnsupported.as_str()
    );
    let token_without_credential = stored_connection(
        GitProviderKind::Forgejo,
        GitConnectionMode::Token,
        "https://forgejo.example",
        false,
        false,
    );
    assert_eq!(
        error_message(
            validated_source(
                &request,
                Some("forgejo-main"),
                Some(&token_without_credential)
            )
            .expect_err("missing credential")
        ),
        GitRepositoryRegistrationRejection::ConnectionCredentialMissing.as_str()
    );
}

#[test]
fn a_forgejo_token_connection_with_a_credential_is_a_valid_registration() {
    let connection = stored_connection(
        GitProviderKind::Forgejo,
        GitConnectionMode::Token,
        "https://forgejo.example",
        true,
        false,
    );
    let source = validated_source(
        &forgejo_request("https://forgejo.example/octo/repo"),
        Some("forgejo-main"),
        Some(&connection),
    )
    .expect("token registration");
    assert_eq!(source.provider, GitProviderKind::Forgejo);
}

#[test]
fn a_github_token_connection_with_a_credential_is_a_valid_registration() {
    let connection = stored_connection(
        GitProviderKind::GitHub,
        GitConnectionMode::Token,
        "https://api.github.com",
        true,
        false,
    );
    let request = request("https://github.com/octo/repo", "HEAD", None);
    let source = validated_source(&request, Some("github-main"), Some(&connection))
        .expect("GitHub PAT registration");
    assert_eq!(source.provider, GitProviderKind::GitHub);
    assert_eq!(source.connection_key.as_deref(), Some("github-main"));
}

#[test]
fn a_github_installation_connection_is_rejected_at_registration() {
    let connection = stored_connection(
        GitProviderKind::GitHub,
        GitConnectionMode::Installation,
        "https://api.github.com",
        true,
        false,
    );
    let request = request("https://github.com/octo/repo", "HEAD", None);
    assert_eq!(
        error_message(
            validated_source(&request, Some("github-main"), Some(&connection))
                .expect_err("installation is unsupported")
        ),
        GitRepositoryRegistrationRejection::ConnectionModeUnsupported.as_str()
    );
}

#[test]
fn github_may_attach_only_a_github_api_origin_connection() {
    let connection = stored_connection(
        GitProviderKind::GitHub,
        GitConnectionMode::Public,
        "https://api.github.com",
        false,
        false,
    );
    let request = request("https://github.com/octo/repo", "HEAD", None);
    assert!(
        validated_source(&request, Some("github-main"), Some(&connection)).is_ok(),
        "the approved GitHub API origin is attachable"
    );
    let foreign = stored_connection(
        GitProviderKind::GitHub,
        GitConnectionMode::Public,
        "https://github.example",
        false,
        false,
    );
    assert_eq!(
        error_message(
            validated_source(&request, Some("github-main"), Some(&foreign))
                .expect_err("foreign API origin")
        ),
        GitRepositoryRegistrationRejection::ConnectionOriginMismatch.as_str()
    );
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
