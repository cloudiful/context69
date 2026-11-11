//! Focused tests for explicit repository connection attachment (issue #681 work
//! unit 4A2).
//!
//! The handlers are a thin Maintainer gate over group-scoped reads and one
//! group-scoped upsert, so the behavior worth pinning here is the decision and
//! mapping logic in front of persistence: which key shapes are refused, which
//! connections are not attachable, that only the connection reference changes,
//! and that the returned projection carries no connection secret reference.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    contracts::{
        Visibility,
        sources::{
            GitCommitCheckpoint, GitConnectionKeyRejection, GitConnectionMode, GitIndexProfile,
            GitIndexStatus, GitProviderKind, GitRepositoryConnectionRequest, GitVersionPolicy,
        },
    },
    db::{
        GitGroupOwnership, NewGitRepositorySource, StoredGitProviderConnection,
        StoredGitRepositorySource,
    },
    domain_errors::DomainError,
};

use super::{check_attachable, connection_update};

fn ownership() -> GitGroupOwnership {
    GitGroupOwnership {
        group_id: 42,
        group_key: "research".to_string(),
        group_path: "research".to_string(),
        visibility: Visibility::Private,
    }
}

fn connection(
    provider: GitProviderKind,
    disabled_at: Option<DateTime<Utc>>,
) -> StoredGitProviderConnection {
    let now = Utc::now();
    StoredGitProviderConnection {
        group: ownership(),
        connection_key: "github-app".to_string(),
        provider,
        mode: GitConnectionMode::Public,
        display_name: "GitHub public".to_string(),
        base_url: "https://api.github.com".to_string(),
        credential_secret_key: None,
        webhook_secret_key: Some("internal/secret/hook-signing".to_string()),
        app_private_key_secret_key: None,
        disabled_at,
        created_at: now,
        updated_at: now,
    }
}

fn source(provider: GitProviderKind) -> StoredGitRepositorySource {
    let now = Utc::now();
    StoredGitRepositorySource {
        group: ownership(),
        repository_key: Uuid::nil(),
        connection_key: None,
        provider,
        canonical_url: "https://github.com/cloudiful/context69".to_string(),
        owner: "cloudiful".to_string(),
        name: "context69".to_string(),
        default_branch: "main".to_string(),
        version: GitVersionPolicy {
            ref_name: "refs/heads/main".to_string(),
            commit_sha: Some("a".repeat(40)),
        },
        include_patterns: Vec::new(),
        exclude_patterns: Vec::new(),
        interval_seconds: None,
        webhook_enabled: false,
        enabled: true,
        retention_count: 2,
        created_by_user_id: None,
        config_revision: 0,
        refresh_dirty: false,
        refresh_requested_at: None,
        last_refresh_attempt_at: None,
        force_rebuild_requested_at: None,
        deletion_requested_at: None,
        last_skip_summary: Default::default(),
        indexed_pipeline_version: 0,
        indexed_config_fingerprint: String::new(),
        index_profile: GitIndexProfile::Hybrid,
        index_status: GitIndexStatus::Ready,
        last_error_code: None,
        checkpoint: GitCommitCheckpoint {
            target_commit_sha: Some("a".repeat(40)),
            indexed_commit_sha: Some("a".repeat(40)),
            indexed_at: Some(now),
            checkpoint_updated_at: Some(now),
        },
        active_generation_key: None,
        created_at: now,
        updated_at: now,
    }
}

fn request(key: &str) -> GitRepositoryConnectionRequest {
    GitRepositoryConnectionRequest {
        connection_key: key.to_string(),
    }
}

#[test]
fn attach_accepts_plain_keys_and_preserves_the_exact_value() {
    for key in ["github-app", "github_app_main", "GitHub.App1", "a"] {
        assert_eq!(
            request(key).validated_connection_key().expect("safe key"),
            key,
            "the stored key must be matched byte for byte, never trimmed"
        );
    }
}

#[test]
fn attach_rejects_blank_oversized_and_unsafe_keys() {
    for key in ["", " ", "\t\n"] {
        assert_eq!(
            request(key).validated_connection_key(),
            Err(GitConnectionKeyRejection::Blank),
            "blank key must be refused: {key:?}"
        );
    }

    let oversized = "k".repeat(crate::contracts::sources::GIT_CONNECTION_KEY_MAX_CHARS + 1);
    assert_eq!(
        request(&oversized).validated_connection_key(),
        Err(GitConnectionKeyRejection::TooLong)
    );

    for key in [
        "internal/secret/read-token",
        "secret:read-token",
        "github app",
        "github\napp",
        "github\u{0}app",
        "github\u{7}app",
        "..",
        ".github-app",
        "github/app",
        "app'--",
        "caf\u{e9}-app",
    ] {
        assert_eq!(
            request(key).validated_connection_key(),
            Err(GitConnectionKeyRejection::Unsafe),
            "unsafe key must be refused: {key:?}"
        );
    }

    // A token-shaped value is well-formed as a key: the charset check is not the
    // secret guard. The group-scoped existence lookup is what keeps such a value
    // from ever being stored, and the returned projection what keeps it from
    // being echoed.
    let token_shaped = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
    assert_eq!(
        request(token_shaped)
            .validated_connection_key()
            .expect("well-formed"),
        token_shaped
    );
}

#[test]
fn attach_refuses_disabled_and_provider_mismatched_connections() {
    let github = source(GitProviderKind::GitHub);
    assert!(check_attachable(&connection(GitProviderKind::GitHub, None), &github).is_ok());

    let disabled = check_attachable(
        &connection(GitProviderKind::GitHub, Some(Utc::now())),
        &github,
    )
    .expect_err("disabled connection must be refused");
    assert_eq!(disabled, DomainError::conflict("git_connection_disabled"));

    let mismatch = check_attachable(&connection(GitProviderKind::GitLab, None), &github)
        .expect_err("provider mismatch must be refused");
    assert_eq!(
        mismatch,
        DomainError::conflict("git_connection_provider_mismatch")
    );
}

#[test]
fn attach_refuses_installation_and_uncredentialed_token_connections() {
    let github = source(GitProviderKind::GitHub);

    let mut installation = connection(GitProviderKind::GitHub, None);
    installation.mode = GitConnectionMode::Installation;
    installation.credential_secret_key = Some("internal/secret/read-token".to_string());
    assert_eq!(
        check_attachable(&installation, &github).expect_err("installation is unsupported"),
        DomainError::conflict("git_connection_mode_unsupported")
    );

    let mut token = connection(GitProviderKind::GitHub, None);
    token.mode = GitConnectionMode::Token;
    assert_eq!(
        check_attachable(&token, &github).expect_err("token needs a credential"),
        DomainError::conflict("git_connection_credential_missing")
    );
}

#[test]
fn attach_refuses_a_connection_whose_origin_does_not_serve_the_repository() {
    let mut forgejo_source = source(GitProviderKind::Forgejo);
    forgejo_source.canonical_url = "https://forgejo.example/octo/repo".to_string();
    let mut forgejo = connection(GitProviderKind::Forgejo, None);
    forgejo.base_url = "https://forgejo.example/api/v1".to_string();
    assert!(
        check_attachable(&forgejo, &forgejo_source).is_ok(),
        "a connection on the repository origin is attachable"
    );
    let foreign = connection(GitProviderKind::Forgejo, None);
    assert_eq!(
        check_attachable(&foreign, &forgejo_source).expect_err("origin mismatch"),
        DomainError::conflict("git_connection_origin_mismatch")
    );
}

#[test]
fn connection_update_changes_only_the_connection_reference() {
    let stored = source(GitProviderKind::GitHub);

    for connection_key in [Some("github-app".to_string()), None] {
        let update = connection_update(&stored, connection_key.clone());
        assert_eq!(update.connection_key, connection_key);
        // Identity, version, and profile come from the stored row, so an attach
        // or detach can never retarget or re-profile the repository.
        assert_eq!(update.provider, stored.provider);
        assert_eq!(update.canonical_url, stored.canonical_url);
        assert_eq!(update.owner, stored.owner);
        assert_eq!(update.name, stored.name);
        assert_eq!(update.default_branch, stored.default_branch);
        assert_eq!(update.target_ref, stored.version.ref_name);
        assert_eq!(update.pinned_commit_sha, stored.version.commit_sha);
        assert_eq!(update.index_profile, stored.index_profile);
    }
}

#[test]
fn attach_returns_a_projection_without_connection_secrets() {
    let mut stored = source(GitProviderKind::GitHub);
    stored.connection_key = Some("github-app".to_string());
    let serialized = serde_json::to_string(&stored.to_contract()).expect("contract serializes");

    assert!(
        !serialized.contains("internal/secret"),
        "connection secret references must never cross the API: {serialized}"
    );
    for forbidden in [
        "credential_secret_key",
        "webhook_secret_key",
        "has_read_credential",
        // The App private key is a separate purpose with no contract field, so an
        // attach projection cannot leak it either.
        "app_private_key_secret_key",
        "github_app.private_key",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "connection secret detail must not be projected: {forbidden}"
        );
    }
    let value: Value = serde_json::from_str(&serialized).expect("valid json");
    assert_eq!(
        value.get("connection_key").and_then(Value::as_str),
        Some("github-app"),
        "the projection reports only the non-secret connection reference"
    );
}

#[test]
fn detach_update_is_a_plain_new_source_row() {
    let stored = source(GitProviderKind::GitHub);
    let update: NewGitRepositorySource = connection_update(&stored, None);
    assert_eq!(update.connection_key, None);
    assert_eq!(update.canonical_url, stored.canonical_url);
}
