//! Focused tests for Git provider connection readiness (issue #681 phase 4B2).
//!
//! The projection is offline and pure: readiness is decided from persisted
//! metadata alone, so these tests construct stored connections in memory and
//! never touch a database, a network, or a secret value. They pin the mode
//! matrix, disabled precedence, the exact non-secret response field set, and
//! the bounded not-found shape an unknown or foreign key shares.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{
    contracts::{
        Visibility,
        sources::{GitConnectionMode, GitConnectionReadiness},
    },
    db::{GitGroupOwnership, StoredGitProviderConnection},
    services::git_repository::connection_readiness::{readiness, response},
};

use super::connection_not_found;

fn ownership() -> GitGroupOwnership {
    GitGroupOwnership {
        group_id: 42,
        group_key: "research".to_string(),
        group_path: "research".to_string(),
        visibility: Visibility::Private,
    }
}

/// A stored connection carrying both secret-store references and the internal
/// App-private-key reference, so every projection test can show that none of
/// them reach the readiness response.
fn stored(
    mode: GitConnectionMode,
    credential_secret_key: Option<&str>,
    disabled_at: Option<DateTime<Utc>>,
) -> StoredGitProviderConnection {
    let now = Utc::now();
    StoredGitProviderConnection {
        group: ownership(),
        connection_key: "github-app".to_string(),
        provider: crate::contracts::sources::GitProviderKind::GitHub,
        mode,
        display_name: "GitHub App".to_string(),
        base_url: "https://api.github.com".to_string(),
        credential_secret_key: credential_secret_key.map(str::to_string),
        webhook_secret_key: Some("internal/secret/hook-signing".to_string()),
        app_private_key_secret_key: Some("github_app.private_key.g42.app".to_string()),
        disabled_at,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn readiness_covers_every_mode_and_credential_state() {
    // Public reads need no credential, so a present or absent reference reads
    // the same.
    assert_eq!(
        readiness(GitConnectionMode::Public, false, false),
        GitConnectionReadiness::Public
    );
    assert_eq!(
        readiness(GitConnectionMode::Public, true, false),
        GitConnectionReadiness::Public
    );

    // Token reads are ready only with a stored read-credential reference.
    assert_eq!(
        readiness(GitConnectionMode::Token, true, false),
        GitConnectionReadiness::Token
    );
    assert_eq!(
        readiness(GitConnectionMode::Token, false, false),
        GitConnectionReadiness::Incomplete
    );

    // Installation reads stay incomplete until a later phase persists an
    // installation identity and transport; a credential reference does not
    // stand in for that identity.
    assert_eq!(
        readiness(GitConnectionMode::Installation, false, false),
        GitConnectionReadiness::Incomplete
    );
    assert_eq!(
        readiness(GitConnectionMode::Installation, true, false),
        GitConnectionReadiness::Incomplete
    );
}

#[test]
fn disabled_precedes_every_other_fact() {
    for mode in [
        GitConnectionMode::Public,
        GitConnectionMode::Token,
        GitConnectionMode::Installation,
    ] {
        for has_read_credential in [false, true] {
            assert_eq!(
                readiness(mode, has_read_credential, true),
                GitConnectionReadiness::Disabled,
                "disabled must win for {mode:?} with credential {has_read_credential}"
            );
        }
    }
}

#[test]
fn response_reports_only_the_non_secret_readiness_fields() {
    let connection = stored(
        GitConnectionMode::Token,
        Some("internal/secret/read-token"),
        None,
    );
    let projected = response(&connection);

    assert_eq!(projected.connection_key, "github-app");
    assert_eq!(projected.mode, GitConnectionMode::Token);
    assert_eq!(projected.readiness, GitConnectionReadiness::Token);
    assert!(projected.has_read_credential);
    assert!(!projected.disabled);

    let encoded = serde_json::to_value(&projected).expect("readiness serializes");
    let object = encoded.as_object().expect("readiness object");
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "connection_key",
            "disabled",
            "has_read_credential",
            "mode",
            "readiness",
        ],
        "the readiness response exposes exactly the five planned non-secret fields"
    );

    let serialized = serde_json::to_string(&projected).expect("readiness serializes");
    for forbidden in [
        "internal/secret",
        "credential_secret_key",
        "webhook_secret_key",
        "app_private_key",
        "github_app.private_key",
        "base_url",
        "group_path",
        "display_name",
        "provider",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "readiness must not project {forbidden}: {serialized}"
        );
    }
}

#[test]
fn installation_reads_incomplete_without_exposing_the_app_key() {
    let connection = stored(GitConnectionMode::Installation, None, None);
    assert!(
        connection.app_private_key_secret_key.is_some(),
        "the fixture must actually carry the App-key reference this test relies on"
    );
    let projected = response(&connection);
    assert_eq!(projected.readiness, GitConnectionReadiness::Incomplete);
    assert!(!projected.has_read_credential);

    let serialized = serde_json::to_string(&projected).expect("readiness serializes");
    assert!(
        !serialized.contains("app_private_key") && !serialized.contains("github_app"),
        "no App-key detail may cross the readiness response: {serialized}"
    );
    let object = serialized_object(&serialized);
    assert!(
        !object.keys().any(|key| key.contains("app")),
        "the readiness contract gained no App-key field: {:?}",
        object.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_disabled_token_connection_reports_disabled_and_keeps_the_credential_fact() {
    let connection = stored(
        GitConnectionMode::Token,
        Some("internal/secret/read-token"),
        Some(Utc::now()),
    );
    let projected = response(&connection);
    assert_eq!(projected.readiness, GitConnectionReadiness::Disabled);
    assert!(projected.disabled);
    assert!(projected.has_read_credential);
    assert_eq!(
        serde_json::to_value(&projected)
            .expect("serialize")
            .get("readiness")
            .and_then(Value::as_str),
        Some("disabled")
    );
}

#[test]
fn existing_connection_projection_is_unchanged_and_never_carries_an_app_key() {
    let connection = stored(
        GitConnectionMode::Installation,
        Some("internal/secret/read-token"),
        None,
    );
    let contract = connection.to_contract();
    let serialized = serde_json::to_string(&contract).expect("connection serializes");
    let object = serialized_object(&serialized);

    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "base_url",
            "connection_key",
            "created_at",
            "disabled",
            "display_name",
            "group_key",
            "group_path",
            "has_read_credential",
            "has_webhook_secret",
            "mode",
            "provider",
            "updated_at",
            "visibility",
        ],
        "the existing GitProviderConnection projection must keep its exact field set"
    );
    assert!(
        !serialized.contains("app_private_key") && !serialized.contains("github_app"),
        "no App-key detail may cross the existing connection projection: {serialized}"
    );
}

#[tokio::test]
async fn unknown_and_foreign_connections_share_one_bounded_not_found() {
    let response = connection_not_found();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read bounded error body");
    let body = String::from_utf8(bytes.to_vec()).expect("utf8 body");
    assert!(
        body.contains("unknown git provider connection"),
        "the shared not-found shape carries the bounded message: {body}"
    );
    assert!(
        !body.contains("internal/secret") && !body.contains("github_app"),
        "the not-found shape must not disclose connection detail: {body}"
    );
}

fn serialized_object(serialized: &str) -> serde_json::Map<String, Value> {
    serde_json::from_str::<Value>(serialized)
        .expect("valid json")
        .as_object()
        .expect("object")
        .clone()
}
