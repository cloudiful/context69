//! Focused contract tests for the read-only Git connection and webhook status
//! routes (issue #681 work unit 4A1).
//!
//! The handlers themselves are a thin viewer gate over group-scoped reads, so
//! the behavior worth pinning here is the projection boundary: the existing
//! stored rows carry secret-store *references*, and the routes answer with the
//! contract projections that report presence booleans only.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    contracts::{
        Visibility,
        sources::{GitConnectionMode, GitProviderKind, GitWebhookOwnership},
    },
    db::{GitGroupOwnership, StoredGitProviderConnection, StoredGitWebhookRegistration},
};

fn ownership() -> GitGroupOwnership {
    GitGroupOwnership {
        group_id: 42,
        group_key: "research".to_string(),
        group_path: "research".to_string(),
        visibility: Visibility::Private,
    }
}

fn stored_connection(
    credential_secret_key: Option<&str>,
    webhook_secret_key: Option<&str>,
    disabled_at: Option<DateTime<Utc>>,
) -> StoredGitProviderConnection {
    let now = Utc::now();
    StoredGitProviderConnection {
        group: ownership(),
        connection_key: "github-app".to_string(),
        provider: GitProviderKind::GitHub,
        mode: GitConnectionMode::Installation,
        display_name: "GitHub App".to_string(),
        base_url: "https://api.github.com".to_string(),
        credential_secret_key: credential_secret_key.map(str::to_string),
        webhook_secret_key: webhook_secret_key.map(str::to_string),
        disabled_at,
        created_at: now,
        updated_at: now,
    }
}

fn stored_webhook(signing_secret_key: Option<&str>) -> StoredGitWebhookRegistration {
    let now = Utc::now();
    StoredGitWebhookRegistration {
        repository_key: Uuid::nil(),
        provider: GitProviderKind::GitHub,
        external_hook_id: "hook-1".to_string(),
        ownership: GitWebhookOwnership::Integration,
        active: true,
        signing_secret_key: signing_secret_key.map(str::to_string),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn connection_projection_reports_presence_not_secret_references() {
    let stored = stored_connection(
        Some("internal/secret/read-token"),
        Some("internal/secret/hook-signing"),
        None,
    );
    let contract = stored.to_contract();

    assert!(contract.has_read_credential);
    assert!(contract.has_webhook_secret);
    assert!(!contract.disabled);
    assert_eq!(contract.group_key, "research");
    assert_eq!(contract.group_path, "research");
    assert_eq!(contract.connection_key, "github-app");
    assert_eq!(contract.mode, GitConnectionMode::Installation);

    let serialized = serde_json::to_string(&contract).expect("contract serializes");
    assert!(
        !serialized.contains("internal/secret"),
        "secret-store references must never cross the API: {serialized}"
    );
    for forbidden in ["credential_secret_key", "webhook_secret_key"] {
        assert!(
            !serialized.contains(forbidden),
            "secret key name must not be projected: {forbidden}"
        );
    }
    let value: Value = serde_json::from_str(&serialized).expect("valid json");
    let object = value.as_object().expect("object");
    assert_eq!(
        object.get("has_read_credential").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        object.get("has_webhook_secret").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn connection_projection_marks_absent_secrets_and_disabled() {
    let contract = stored_connection(None, None, Some(Utc::now())).to_contract();

    assert!(!contract.has_read_credential);
    assert!(!contract.has_webhook_secret);
    assert!(contract.disabled);
}

#[test]
fn webhook_projection_exposes_only_secret_presence() {
    let with_secret = stored_webhook(Some("internal/secret/signing")).to_contract();
    assert!(with_secret.has_signing_secret);
    assert!(with_secret.active);
    assert_eq!(with_secret.ownership, GitWebhookOwnership::Integration);

    let serialized = serde_json::to_string(&with_secret).expect("contract serializes");
    assert!(
        !serialized.contains("internal/secret") && !serialized.contains("signing_secret_key"),
        "signing secret reference must never cross the API: {serialized}"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&serialized)
            .expect("valid json")
            .get("has_signing_secret")
            .and_then(Value::as_bool),
        Some(true)
    );

    let without_secret = stored_webhook(None).to_contract();
    assert!(!without_secret.has_signing_secret);
}
