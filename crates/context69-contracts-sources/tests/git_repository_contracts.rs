//! Provider-neutral Git source and connection contracts (issue #681 phases 2,
//! 3B1, 4A2, 4B2, and 4B3).
//!
//! Covers the wire shape of the contract enums/structs and the shape of
//! `migrations/20260930204952_git_repository_sources.sql`: the migration must
//! keep secret material as references to `context69.internal_secrets`, carry
//! the target/indexed commit checkpoint, and key webhook deliveries by the
//! provider delivery id so redelivery is idempotent. The group-ownership,
//! generation, and query invariants of
//! `migrations/20261001010156_git_repository_groups_generations.sql` are
//! asserted next to the persistence layer that depends on them, in
//! `src/db/git_repositories/schema_tests.rs`.

use chrono::{DateTime, Utc};
use context69_contracts_core::Visibility;
use context69_contracts_sources::{
    GIT_CONNECTION_BASE_URL_MAX_CHARS, GIT_CONNECTION_DISPLAY_NAME_MAX_CHARS,
    GIT_CONNECTION_KEY_MAX_CHARS, GitActiveGeneration, GitCommitCheckpoint,
    GitConnectionKeyRejection, GitConnectionMode, GitConnectionReadiness,
    GitConnectionReadinessResponse, GitConnectionRequestRejection, GitGenerationStatus,
    GitIndexProfile, GitIndexStatus, GitProviderConnection, GitProviderConnectionRequest,
    GitProviderKind, GitReadCredentialPatch, GitRefreshPolicy, GitRepositoryConnectionRequest,
    GitRepositoryGeneration, GitRepositoryRegistrationRequest, GitRepositorySource,
    GitVersionPolicy, GitWebhookDelivery, GitWebhookDeliveryStatus, GitWebhookOwnership,
    GitWebhookRegistration, validate_git_connection_key,
};
use schemars::schema_for;
use serde_json::{from_value, json, to_value};
use uuid::Uuid;

const MIGRATION_SQL: &str =
    include_str!("../../../migrations/20260930204952_git_repository_sources.sql");

fn timestamp() -> DateTime<Utc> {
    "2026-09-30T12:00:00Z".parse().expect("timestamp")
}

fn repository_key() -> Uuid {
    Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid")
}

fn sample_source() -> GitRepositorySource {
    GitRepositorySource {
        group_key: "team-platform".to_string(),
        group_path: "acme/team-platform".to_string(),
        visibility: Visibility::Private,
        repository_key: repository_key(),
        connection_key: Some("github-app-main".to_string()),
        provider: GitProviderKind::GitHub,
        canonical_url: "https://github.com/cloudiful/context69".to_string(),
        owner: "cloudiful".to_string(),
        name: "context69".to_string(),
        default_branch: "main".to_string(),
        version: GitVersionPolicy {
            ref_name: "main".to_string(),
            commit_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
        },
        refresh: GitRefreshPolicy::Webhook,
        index_profile: GitIndexProfile::Hybrid,
        index_status: GitIndexStatus::Ready,
        checkpoint: GitCommitCheckpoint {
            target_commit_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            indexed_commit_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            indexed_at: Some(timestamp()),
            checkpoint_updated_at: Some(timestamp()),
        },
        active_generation_key: Some(
            Uuid::parse_str("018f9f3b-0000-7000-8000-0000000000aa").expect("uuid"),
        ),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

#[test]
fn git_enum_wire_names_are_stable() {
    assert_eq!(
        to_value([
            GitProviderKind::GitHub,
            GitProviderKind::Forgejo,
            GitProviderKind::GitLab,
            GitProviderKind::Generic,
        ])
        .expect("serialize provider kinds"),
        json!(["github", "forgejo", "gitlab", "generic"])
    );
    assert_eq!(
        to_value([
            GitConnectionMode::Public,
            GitConnectionMode::Installation,
            GitConnectionMode::Token,
        ])
        .expect("serialize connection modes"),
        json!(["public", "installation", "token"])
    );
    assert_eq!(
        to_value([
            GitRefreshPolicy::Manual,
            GitRefreshPolicy::Webhook,
            GitRefreshPolicy::Reconcile,
        ])
        .expect("serialize refresh policies"),
        json!(["manual", "webhook", "reconcile"])
    );
    assert_eq!(
        to_value([
            GitIndexProfile::Lexical,
            GitIndexProfile::Hybrid,
            GitIndexProfile::FullSemantic,
        ])
        .expect("serialize index profiles"),
        json!(["lexical", "hybrid", "full_semantic"])
    );
    assert_eq!(
        to_value([
            GitIndexStatus::Pending,
            GitIndexStatus::Indexing,
            GitIndexStatus::Ready,
            GitIndexStatus::Stale,
            GitIndexStatus::Failed,
            GitIndexStatus::Disabled,
        ])
        .expect("serialize index statuses"),
        json!([
            "pending", "indexing", "ready", "stale", "failed", "disabled"
        ])
    );
    assert_eq!(
        to_value([
            GitWebhookOwnership::Integration,
            GitWebhookOwnership::External,
            GitWebhookOwnership::Unknown,
        ])
        .expect("serialize ownership"),
        json!(["integration", "external", "unknown"])
    );
    assert_eq!(
        to_value([
            GitWebhookDeliveryStatus::Received,
            GitWebhookDeliveryStatus::Queued,
            GitWebhookDeliveryStatus::Ignored,
            GitWebhookDeliveryStatus::Failed,
        ])
        .expect("serialize delivery statuses"),
        json!(["received", "queued", "ignored", "failed"])
    );
    assert_eq!(
        to_value([
            GitGenerationStatus::Building,
            GitGenerationStatus::Ready,
            GitGenerationStatus::Failed,
            GitGenerationStatus::Superseded,
        ])
        .expect("serialize generation statuses"),
        json!(["building", "ready", "failed", "superseded"])
    );
}

#[test]
fn repository_source_round_trips_identity_policies_and_checkpoint() {
    let source = sample_source();
    let encoded = to_value(&source).expect("serialize source");
    assert_eq!(encoded["provider"], json!("github"));
    assert_eq!(encoded["refresh"], json!("webhook"));
    assert_eq!(encoded["index_profile"], json!("hybrid"));
    assert_eq!(encoded["group_key"], json!("team-platform"));
    assert_eq!(encoded["group_path"], json!("acme/team-platform"));
    assert_eq!(encoded["visibility"], json!("private"));
    assert_eq!(encoded["version"]["ref_name"], json!("main"));
    assert_eq!(
        encoded["active_generation_key"],
        json!("018f9f3b-0000-7000-8000-0000000000aa")
    );
    assert_eq!(
        encoded["checkpoint"]["indexed_commit_sha"],
        json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );

    let decoded: GitRepositorySource = from_value(encoded).expect("deserialize source");
    assert_eq!(decoded, source);
}

#[test]
fn repository_source_tolerates_absent_optional_fields() {
    let source: GitRepositorySource = from_value(json!({
        "group_key": "team-platform",
        "group_path": "acme/team-platform",
        "visibility": "public",
        "repository_key": "018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f",
        "provider": "forgejo",
        "canonical_url": "https://code.example.test/team/repo",
        "owner": "team",
        "name": "repo",
        "default_branch": "main",
        "version": { "ref_name": "main" },
        "refresh": "reconcile",
        "index_profile": "lexical",
        "index_status": "pending",
        "checkpoint": {},
        "created_at": "2026-09-30T12:00:00Z",
        "updated_at": "2026-09-30T12:00:00Z"
    }))
    .expect("payload without optional fields");

    assert_eq!(source.connection_key, None);
    assert_eq!(source.version.commit_sha, None);
    assert_eq!(source.active_generation_key, None);
    assert_eq!(source.visibility, Visibility::Public);
    assert_eq!(
        source.checkpoint,
        GitCommitCheckpoint {
            target_commit_sha: None,
            indexed_commit_sha: None,
            indexed_at: None,
            checkpoint_updated_at: None,
        }
    );
}

#[test]
fn checkpoint_keeps_target_and_indexed_commits_distinct() {
    let checkpoint = GitCommitCheckpoint {
        target_commit_sha: Some("bbbb".to_string()),
        indexed_commit_sha: Some("aaaa".to_string()),
        indexed_at: Some(timestamp()),
        checkpoint_updated_at: None,
    };
    let encoded = to_value(&checkpoint).expect("serialize checkpoint");
    assert_eq!(encoded["target_commit_sha"], json!("bbbb"));
    assert_eq!(encoded["indexed_commit_sha"], json!("aaaa"));
    assert!(encoded.get("checkpoint_updated_at").is_none());

    let decoded: GitCommitCheckpoint = from_value(encoded).expect("deserialize checkpoint");
    assert_eq!(decoded, checkpoint);
}

#[test]
fn connection_contract_exposes_only_secret_presence() {
    let connection = GitProviderConnection {
        group_key: "team-platform".to_string(),
        group_path: "acme/team-platform".to_string(),
        visibility: Visibility::Private,
        connection_key: "github-app-main".to_string(),
        provider: GitProviderKind::GitHub,
        mode: GitConnectionMode::Installation,
        display_name: "GitHub App".to_string(),
        base_url: "https://github.com".to_string(),
        has_read_credential: true,
        has_webhook_secret: true,
        disabled: false,
        created_at: timestamp(),
        updated_at: timestamp(),
    };
    let encoded = to_value(&connection).expect("serialize connection");
    let object = encoded.as_object().expect("connection object");
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
        "connection contract must expose the owning group and presence flags, never secret material"
    );
    assert_eq!(object["has_read_credential"], json!(true));
    assert_eq!(object["has_webhook_secret"], json!(true));

    let decoded: GitProviderConnection = from_value(encoded).expect("deserialize connection");
    assert_eq!(decoded, connection);
}

#[test]
fn connection_readiness_wire_names_are_stable() {
    assert_eq!(
        to_value([
            GitConnectionReadiness::Public,
            GitConnectionReadiness::Token,
            GitConnectionReadiness::Installation,
            GitConnectionReadiness::Incomplete,
            GitConnectionReadiness::Disabled,
        ])
        .expect("serialize readiness"),
        json!(["public", "token", "installation", "incomplete", "disabled"])
    );
    for (value, name) in [
        (GitConnectionReadiness::Public, "public"),
        (GitConnectionReadiness::Token, "token"),
        (GitConnectionReadiness::Installation, "installation"),
        (GitConnectionReadiness::Incomplete, "incomplete"),
        (GitConnectionReadiness::Disabled, "disabled"),
    ] {
        assert_eq!(value.as_str(), name);
    }
}

#[test]
fn readiness_response_exposes_only_the_planned_non_secret_fields() {
    let response = GitConnectionReadinessResponse {
        connection_key: "github-app-main".to_string(),
        mode: GitConnectionMode::Token,
        readiness: GitConnectionReadiness::Token,
        has_read_credential: true,
        disabled: false,
    };
    let encoded = to_value(&response).expect("serialize readiness response");
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
        "readiness must expose exactly the planned non-secret fields"
    );
    assert_eq!(object["mode"], json!("token"));
    assert_eq!(object["readiness"], json!("token"));
    assert_eq!(object["has_read_credential"], json!(true));
    assert_eq!(object["disabled"], json!(false));
    for forbidden in [
        "app_id",
        "app_private_key",
        "installation_id",
        "credential_secret_key",
        "webhook_secret_key",
        "base_url",
        "display_name",
        "provider",
        "group_path",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "readiness must not carry {forbidden}"
        );
    }

    let decoded: GitConnectionReadinessResponse =
        from_value(encoded).expect("deserialize readiness response");
    assert_eq!(decoded, response);
}

#[test]
fn webhook_registration_and_delivery_round_trip() {
    let registration = GitWebhookRegistration {
        repository_key: repository_key(),
        provider: GitProviderKind::GitHub,
        external_hook_id: "hook-42".to_string(),
        ownership: GitWebhookOwnership::Integration,
        active: true,
        has_signing_secret: true,
        created_at: timestamp(),
        updated_at: timestamp(),
    };
    let encoded = to_value(&registration).expect("serialize registration");
    assert_eq!(encoded["ownership"], json!("integration"));
    assert_eq!(encoded["has_signing_secret"], json!(true));
    let decoded: GitWebhookRegistration = from_value(encoded).expect("deserialize registration");
    assert_eq!(decoded, registration);

    let delivery = GitWebhookDelivery {
        delivery_id: "delivery-7".to_string(),
        repository_key: Some(repository_key()),
        provider: GitProviderKind::GitHub,
        status: GitWebhookDeliveryStatus::Received,
        target_commit_sha: Some("cccc".to_string()),
        received_at: timestamp(),
        processed_at: None,
    };
    let encoded = to_value(&delivery).expect("serialize delivery");
    assert_eq!(encoded["delivery_id"], json!("delivery-7"));
    assert_eq!(encoded["status"], json!("received"));
    assert!(encoded.get("processed_at").is_none());
    let decoded: GitWebhookDelivery = from_value(encoded).expect("deserialize delivery");
    assert_eq!(decoded, delivery);
}

fn table_body<'a>(sql: &'a str, table: &str) -> &'a str {
    let marker = format!("CREATE TABLE IF NOT EXISTS context69.{table} (");
    let start = sql
        .find(&marker)
        .unwrap_or_else(|| panic!("migration must create {table}"));
    let rest = &sql[start + marker.len()..];
    let mut depth = 1_i32;
    for (index, character) in rest.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..index];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated CREATE TABLE for {table}");
}

fn has_column(body: &str, name: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    })
}

fn assert_columns(table: &str, body: &str, columns: &[&str]) {
    for column in columns {
        assert!(
            has_column(body, column),
            "{table} must define column {column}"
        );
    }
}

fn secret_column_lines(body: &str) -> Vec<&str> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("CONSTRAINT") && !line.starts_with("CHECK"))
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("secret") || lower.contains("token") || lower.contains("password")
        })
        .collect()
}

fn assert_secret_columns_are_references(table: &str, body: &str) {
    for line in secret_column_lines(body) {
        assert!(
            line.contains("REFERENCES context69.internal_secrets(key)"),
            "{table} secret column must reference the internal secret store: {line}"
        );
    }
}

#[test]
fn migration_defines_git_tables_and_columns() {
    assert_columns(
        "git_provider_connections",
        table_body(MIGRATION_SQL, "git_provider_connections"),
        &[
            "connection_key",
            "provider_kind",
            "connection_mode",
            "display_name",
            "base_url",
            "credential_secret_key",
            "webhook_secret_key",
            "disabled_at",
            "created_at",
            "updated_at",
        ],
    );
    assert_columns(
        "git_repository_sources",
        table_body(MIGRATION_SQL, "git_repository_sources"),
        &[
            "repository_key",
            "connection_key",
            "provider_kind",
            "canonical_url",
            "repository_owner",
            "repository_name",
            "default_branch",
            "target_ref",
            "target_commit_sha",
            "indexed_commit_sha",
            "index_profile",
            "refresh_policy",
            "index_status",
            "last_indexed_at",
            "checkpoint_updated_at",
        ],
    );
    assert_columns(
        "git_webhook_registrations",
        table_body(MIGRATION_SQL, "git_webhook_registrations"),
        &[
            "repository_key",
            "provider_kind",
            "external_hook_id",
            "ownership",
            "active",
            "signing_secret_key",
        ],
    );
    assert_columns(
        "git_webhook_deliveries",
        table_body(MIGRATION_SQL, "git_webhook_deliveries"),
        &[
            "delivery_id",
            "provider_kind",
            "repository_key",
            "status",
            "target_commit_sha",
            "received_at",
            "processed_at",
        ],
    );
}

#[test]
fn migration_keeps_secrets_as_internal_secret_references() {
    for table in ["git_provider_connections", "git_webhook_registrations"] {
        let body = table_body(MIGRATION_SQL, table);
        assert_secret_columns_are_references(table, body);
        assert!(
            !secret_column_lines(body).is_empty(),
            "{table} must reference at least one secret"
        );
    }
    assert!(
        secret_column_lines(table_body(MIGRATION_SQL, "git_webhook_deliveries")).is_empty(),
        "webhook deliveries carry no secret material"
    );
    let lower = MIGRATION_SQL.to_ascii_lowercase();
    for forbidden in ["access_token", "password", "private_key"] {
        assert!(
            !lower.contains(forbidden),
            "migration must not persist plaintext {forbidden}"
        );
    }
}

#[test]
fn migration_keys_identity_checkpoint_and_delivery_idempotency() {
    let repository = table_body(MIGRATION_SQL, "git_repository_sources");
    assert!(
        repository.contains("UNIQUE (canonical_url, target_ref)"),
        "one source row per repository ref"
    );
    assert!(
        repository.contains(
            "index_status IN ('pending', 'indexing', 'ready', 'stale', 'failed', 'disabled')"
        ),
        "index status is constrained"
    );

    let deliveries = table_body(MIGRATION_SQL, "git_webhook_deliveries");
    assert!(
        has_column(deliveries, "delivery_id")
            && deliveries.contains("delivery_id TEXT PRIMARY KEY"),
        "webhook deliveries are keyed by provider delivery id"
    );

    assert!(
        MIGRATION_SQL.contains("UNIQUE INDEX IF NOT EXISTS uq_git_webhook_registrations_hook")
            && MIGRATION_SQL.contains("(provider_kind, external_hook_id)"),
        "a provider hook id can be owned by one repository only"
    );
}

#[test]
fn migration_secret_columns_are_optional_references() {
    let body = table_body(MIGRATION_SQL, "git_provider_connections");
    let secret_lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("_secret_key"))
        .collect();
    assert_eq!(secret_lines.len(), 2, "read credential and webhook secret");
    for line in secret_lines {
        assert!(
            !line.contains("NOT NULL"),
            "secret references must stay optional for public sources: {line}"
        );
    }
}

#[test]
fn migration_has_no_unexpected_extra_tables() {
    let created = MIGRATION_SQL
        .match_indices("CREATE TABLE IF NOT EXISTS context69.")
        .map(|(index, marker)| {
            let rest = &MIGRATION_SQL[index + marker.len()..];
            let end = rest.find([' ', '(']).unwrap_or(rest.len());
            rest[..end].to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        created,
        vec![
            "git_provider_connections",
            "git_repository_sources",
            "git_webhook_registrations",
            "git_webhook_deliveries",
        ]
    );
}

#[test]
fn generation_contracts_round_trip_metadata_and_pointer() {
    let generation = GitRepositoryGeneration {
        generation_key: Uuid::parse_str("018f9f3c-1111-7000-8000-0000000000bb").expect("uuid"),
        repository_key: repository_key(),
        generation_number: 7,
        ref_name: "main".to_string(),
        commit_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        index_profile: GitIndexProfile::Lexical,
        status: GitGenerationStatus::Ready,
        file_count: 120,
        excluded_file_count: 3,
        total_bytes: 4096,
        error_code: None,
        started_at: timestamp(),
        completed_at: Some(timestamp()),
        created_at: timestamp(),
        updated_at: timestamp(),
    };
    let encoded = to_value(&generation).expect("serialize generation");
    assert_eq!(encoded["status"], json!("ready"));
    assert_eq!(encoded["generation_number"], json!(7));
    assert_eq!(encoded["index_profile"], json!("lexical"));
    assert_eq!(
        encoded["excluded_file_count"],
        json!(3),
        "coverage gaps must be visible on the generation"
    );
    let decoded: GitRepositoryGeneration = from_value(encoded).expect("deserialize generation");
    assert_eq!(decoded, generation);

    let active = GitActiveGeneration {
        repository_key: repository_key(),
        generation_key: generation.generation_key,
        activated_at: timestamp(),
    };
    let encoded = to_value(&active).expect("serialize active generation");
    assert_eq!(encoded["generation_key"], json!(active.generation_key));
    let decoded: GitActiveGeneration = from_value(encoded).expect("deserialize active generation");
    assert_eq!(decoded, active);
}

#[test]
fn generation_contract_tolerates_absent_optional_fields() {
    let generation: GitRepositoryGeneration = from_value(json!({
        "generation_key": "018f9f3c-1111-7000-8000-0000000000bb",
        "repository_key": "018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f",
        "generation_number": 1,
        "ref_name": "main",
        "commit_sha": "cccccccccccccccccccccccccccccccccccccccc",
        "index_profile": "lexical",
        "status": "building",
        "file_count": 0,
        "excluded_file_count": 0,
        "total_bytes": 0,
        "started_at": "2026-09-30T12:00:00Z",
        "created_at": "2026-09-30T12:00:00Z",
        "updated_at": "2026-09-30T12:00:00Z"
    }))
    .expect("building generation without optional fields");
    assert_eq!(generation.status, GitGenerationStatus::Building);
    assert_eq!(generation.error_code, None);
    assert_eq!(generation.completed_at, None);
}

#[test]
fn registration_request_defaults_policies_without_optional_fields() {
    let request: GitRepositoryRegistrationRequest = from_value(json!({
        "canonical_url": "https://github.com/cloudiful/context69",
        "default_branch": "main",
        "target_ref": "refs/heads/main"
    }))
    .expect("minimal registration request");

    assert_eq!(request.index_profile, GitIndexProfile::Lexical);
    assert_eq!(request.refresh_policy, GitRefreshPolicy::Manual);
    assert_eq!(request.pinned_commit, None);
}

#[test]
fn registration_request_round_trips_pin_and_policies_without_secret_fields() {
    let request = GitRepositoryRegistrationRequest {
        canonical_url: "https://github.com/cloudiful/context69".to_string(),
        default_branch: "main".to_string(),
        target_ref: "refs/tags/v1.2.3".to_string(),
        pinned_commit: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
        index_profile: GitIndexProfile::Hybrid,
        refresh_policy: GitRefreshPolicy::Webhook,
    };
    let encoded = to_value(&request).expect("serialize request");
    assert_eq!(encoded["index_profile"], json!("hybrid"));
    assert_eq!(encoded["refresh_policy"], json!("webhook"));
    assert_eq!(
        encoded["pinned_commit"],
        json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );

    // Registration carries no credential or provider-connection material.
    let object = encoded.as_object().expect("request object");
    for forbidden in [
        "connection_key",
        "credential_secret_key",
        "webhook_secret_key",
        "access_token",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "registration request must not carry {forbidden}"
        );
    }

    let decoded: GitRepositoryRegistrationRequest =
        from_value(encoded).expect("deserialize request");
    assert_eq!(decoded, request);
}

#[test]
fn registration_request_omits_absent_optional_pin() {
    let request = GitRepositoryRegistrationRequest {
        canonical_url: "https://github.com/cloudiful/context69".to_string(),
        default_branch: "main".to_string(),
        target_ref: "HEAD".to_string(),
        pinned_commit: None,
        index_profile: GitIndexProfile::Lexical,
        refresh_policy: GitRefreshPolicy::Manual,
    };
    let encoded = to_value(&request).expect("serialize request");
    assert!(encoded.get("pinned_commit").is_none());
    assert_eq!(encoded["index_profile"], json!("lexical"));
    assert_eq!(encoded["refresh_policy"], json!("manual"));
}

#[test]
fn connection_request_carries_only_the_existing_connection_key() {
    let request = GitRepositoryConnectionRequest {
        connection_key: "github-app-main".to_string(),
    };
    let encoded = to_value(&request).expect("serialize request");
    assert_eq!(encoded, json!({ "connection_key": "github-app-main" }));

    // No credential, token, secret-store key, or other connection field may ride
    // along: attaching metadata can never configure or create a connection.
    let object = encoded.as_object().expect("request object");
    for forbidden in [
        "connection_mode",
        "mode",
        "base_url",
        "display_name",
        "credential_secret_key",
        "webhook_secret_key",
        "access_token",
        "disabled",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "connection request must not carry {forbidden}"
        );
    }
    for extra in [
        json!({ "connection_key": "github-app", "access_token": "ghp_secret" }),
        json!({ "connection_key": "github-app", "credential_secret_key": "internal/secret/read" }),
        json!({ "connection_key": "github-app", "disabled": false }),
    ] {
        assert!(
            from_value::<GitRepositoryConnectionRequest>(extra.clone()).is_err(),
            "unknown connection fields must be rejected: {extra}"
        );
    }
    assert!(
        from_value::<GitRepositoryConnectionRequest>(json!({})).is_err(),
        "connection_key is required"
    );

    let decoded: GitRepositoryConnectionRequest = from_value(encoded).expect("deserialize request");
    assert_eq!(decoded, request);
}

#[test]
fn connection_request_schema_declares_the_key_bounds() {
    let schema = schema_for!(GitRepositoryConnectionRequest);
    let key = schema
        .get("properties")
        .and_then(|properties| properties.get("connection_key"))
        .expect("connection_key property");
    assert_eq!(
        key.get("type").and_then(|kind| kind.as_str()),
        Some("string")
    );
    assert_eq!(key.get("minLength").and_then(|min| min.as_u64()), Some(1));
    assert_eq!(
        key.get("maxLength").and_then(|max| max.as_u64()),
        Some(GIT_CONNECTION_KEY_MAX_CHARS as u64),
    );
    assert!(
        schema.get("required").is_some(),
        "the runtime validator requires connection_key, so the schema must too"
    );
}

#[test]
fn connection_request_validates_key_bounds_without_a_lookup() {
    let accepted = GitRepositoryConnectionRequest {
        connection_key: "github_app-main.1".to_string(),
    };
    assert_eq!(
        accepted.validated_connection_key().expect("safe key"),
        "github_app-main.1"
    );

    for (key, expected) in [
        ("", GitConnectionKeyRejection::Blank),
        ("  ", GitConnectionKeyRejection::Blank),
    ] {
        let request = GitRepositoryConnectionRequest {
            connection_key: key.to_string(),
        };
        assert_eq!(request.validated_connection_key(), Err(expected));
    }

    let oversized = "k".repeat(GIT_CONNECTION_KEY_MAX_CHARS + 1);
    assert_eq!(
        GitRepositoryConnectionRequest {
            connection_key: oversized
        }
        .validated_connection_key(),
        Err(GitConnectionKeyRejection::TooLong)
    );
    let longest_allowed = "k".repeat(GIT_CONNECTION_KEY_MAX_CHARS);
    assert!(
        GitRepositoryConnectionRequest {
            connection_key: longest_allowed
        }
        .validated_connection_key()
        .is_ok()
    );

    // A secret-store reference and a path/control form are both refused: the
    // accepted charset has no separator or whitespace, so secret material can
    // never be submitted as a connection key.
    for unsafe_key in [
        "internal/secret/read-token",
        "secret:read-token",
        "github app",
        "github\napp",
        "..",
        ".github-app",
    ] {
        assert_eq!(
            GitRepositoryConnectionRequest {
                connection_key: unsafe_key.to_string()
            }
            .validated_connection_key(),
            Err(GitConnectionKeyRejection::Unsafe),
            "unsafe key must be refused: {unsafe_key:?}"
        );
    }

    // A token-shaped value is indistinguishable from a key by shape alone, so
    // the charset check does not claim to be the secret guard: it is the
    // group-scoped existence lookup that keeps such a value from ever being
    // stored, and the projection that keeps it from ever being disclosed.
    let token_shaped = GitRepositoryConnectionRequest {
        connection_key: "ghp_16C7e42F292c6912E7710c838347Ae178B4a".to_string(),
    };
    assert_eq!(
        token_shaped
            .validated_connection_key()
            .expect("well-formed key"),
        "ghp_16C7e42F292c6912E7710c838347Ae178B4a"
    );
    // The create path key shares the exact validator, so both request types
    // accept and refuse the same keys without duplicating the charset. The
    // sample keys are descriptive, never secret-store or credential shaped.
    for key in ["github-app", "", "bad key", "..", "caf\u{e9}"] {
        assert_eq!(
            validate_git_connection_key(key),
            GitRepositoryConnectionRequest {
                connection_key: key.to_string()
            }
            .validated_connection_key(),
            "the shared validator must agree on {key:?}"
        );
    }
}

fn create_request() -> GitProviderConnectionRequest {
    GitProviderConnectionRequest {
        provider: GitProviderKind::GitHub,
        mode: GitConnectionMode::Token,
        display_name: "GitHub PAT".to_string(),
        base_url: "https://api.github.com".to_string(),
        read_credential: GitReadCredentialPatch::Keep,
    }
}

#[test]
fn connection_create_request_defaults_to_keep_and_rejects_unknown_fields() {
    let minimal: GitProviderConnectionRequest = from_value(json!({
        "provider": "github",
        "mode": "token",
        "display_name": "GitHub PAT",
        "base_url": "https://api.github.com"
    }))
    .expect("minimal create body");
    assert_eq!(minimal.read_credential, GitReadCredentialPatch::Keep);
    assert!(minimal.validate_for_create().is_ok());

    for missing in ["provider", "mode", "display_name", "base_url"] {
        let mut body = json!({
            "provider": "github",
            "mode": "token",
            "display_name": "GitHub PAT",
            "base_url": "https://api.github.com"
        });
        body.as_object_mut().expect("object").remove(missing);
        assert!(
            from_value::<GitProviderConnectionRequest>(body).is_err(),
            "{missing} is required"
        );
    }

    // No secret-store reference, App key, webhook, or installation field may
    // ride along: creating a connection can never name someone else's secret.
    // The unknown field is rejected on its name alone, so every sample carries
    // an inert placeholder and the assertion never echoes a payload.
    for field in [
        "credential_secret_key",
        "app_private_key",
        "webhook_secret_key",
        "installation_id",
    ] {
        let mut body = json!({
            "provider": "github",
            "mode": "token",
            "display_name": "GitHub PAT",
            "base_url": "https://api.github.com"
        });
        body.as_object_mut()
            .expect("object")
            .insert(field.to_string(), json!("placeholder"));
        assert!(
            from_value::<GitProviderConnectionRequest>(body).is_err(),
            "unknown create field {field} must be rejected"
        );
    }
}

#[test]
fn connection_create_request_round_trips_the_read_credential_patch() {
    // A generated synthetic value, so no credential-shaped literal is embedded
    // in the suite while the exact `set` wire shape is still asserted.
    let synthetic = format!("synthetic-{}", Uuid::new_v4());
    for (label, patch, expected) in [
        ("keep", GitReadCredentialPatch::Keep, json!({"op": "keep"})),
        (
            "set",
            GitReadCredentialPatch::Set(synthetic.clone()),
            json!({"op": "set", "value": synthetic.clone()}),
        ),
        (
            "clear",
            GitReadCredentialPatch::Clear,
            json!({"op": "clear"}),
        ),
    ] {
        let mut request = create_request();
        request.read_credential = patch.clone();
        let encoded = to_value(&request).expect("serialize create request");
        assert!(
            encoded.get("read_credential") == Some(&expected),
            "the {label} patch must keep its wire shape"
        );
        let decoded: GitProviderConnectionRequest =
            from_value(encoded).expect("deserialize create request");
        assert!(
            decoded.read_credential == patch,
            "the {label} patch must round-trip"
        );
    }

    // A missing patch is Keep, never a rotate or clear, and only a non-blank
    // Set yields a value for the server-side writer.
    let request: GitProviderConnectionRequest = from_value(json!({
        "provider": "github",
        "mode": "token",
        "display_name": "GitHub PAT",
        "base_url": "https://api.github.com"
    }))
    .expect("body without read_credential");
    assert!(request.read_credential.is_keep());
    assert!(!request.read_credential.is_clear());
    assert_eq!(request.read_credential.set_value(), None);
    // A non-blank `Set` yields the value for the writer; a blank one is refused
    // by `validate_for_create` before this point and yields nothing here.
    let synthetic = format!("synthetic-{}", Uuid::new_v4());
    let set = GitReadCredentialPatch::Set(synthetic.clone());
    assert!(set.set_value() == Some(synthetic.as_str()));
    assert_eq!(
        GitReadCredentialPatch::Set("   ".to_string()).set_value(),
        None
    );
}

#[test]
fn connection_create_request_validation_is_bounded_and_secret_free() {
    assert!(create_request().validate_for_create().is_ok());

    let cases: Vec<(GitProviderConnectionRequest, GitConnectionRequestRejection)> = vec![
        (
            GitProviderConnectionRequest {
                display_name: "   ".to_string(),
                ..create_request()
            },
            GitConnectionRequestRejection::DisplayNameBlank,
        ),
        (
            GitProviderConnectionRequest {
                display_name: "d".repeat(GIT_CONNECTION_DISPLAY_NAME_MAX_CHARS + 1),
                ..create_request()
            },
            GitConnectionRequestRejection::DisplayNameTooLong,
        ),
        (
            GitProviderConnectionRequest {
                base_url: "  ".to_string(),
                ..create_request()
            },
            GitConnectionRequestRejection::BaseUrlBlank,
        ),
        (
            GitProviderConnectionRequest {
                base_url: "h".repeat(GIT_CONNECTION_BASE_URL_MAX_CHARS + 1),
                ..create_request()
            },
            GitConnectionRequestRejection::BaseUrlTooLong,
        ),
        (
            GitProviderConnectionRequest {
                base_url: "file:///etc/passwd".to_string(),
                ..create_request()
            },
            GitConnectionRequestRejection::BaseUrlUnsupported,
        ),
        (
            GitProviderConnectionRequest {
                read_credential: GitReadCredentialPatch::Set("   ".to_string()),
                ..create_request()
            },
            GitConnectionRequestRejection::CredentialSetBlank,
        ),
        (
            GitProviderConnectionRequest {
                read_credential: GitReadCredentialPatch::Clear,
                ..create_request()
            },
            GitConnectionRequestRejection::CredentialClearUnsupported,
        ),
    ];
    for (request, expected) in cases {
        let rejection = request.validate_for_create().expect_err("must be refused");
        assert_eq!(rejection, expected);
        // The bounded reason is a stable code that never echoes the submitted
        // value, so it is safe to surface in an error body or an assertion.
        let reason = rejection.as_str();
        assert!(
            reason.starts_with("git_connection_") && reason.is_ascii(),
            "the reason is a bounded stable code"
        );
        for forbidden in ["https://", "file://", "   "] {
            assert!(
                !reason.contains(forbidden),
                "reason must not echo a submitted value"
            );
        }
    }
    assert_eq!(
        GitConnectionRequestRejection::CredentialClearUnsupported.as_str(),
        "git_connection_secret_clear_unsupported_on_create"
    );
    assert_eq!(
        GitConnectionRequestRejection::CredentialSetBlank.as_str(),
        "git_connection_secret_set_blank"
    );
}

#[test]
fn connection_create_request_schema_declares_metadata_and_the_patch() {
    let schema = schema_for!(GitProviderConnectionRequest);
    let properties = schema
        .get("properties")
        .and_then(|properties| properties.as_object())
        .expect("create properties");
    let mut keys = properties.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "base_url",
            "display_name",
            "mode",
            "provider",
            "read_credential",
        ],
        "the create schema exposes exactly the planned non-secret fields"
    );
    assert_eq!(
        schema
            .get("additionalProperties")
            .and_then(|value| value.as_bool()),
        Some(false),
        "deny_unknown_fields must be reflected in the schema"
    );
    let required = schema
        .get("required")
        .and_then(|required| required.as_array())
        .expect("required fields");
    let required: Vec<&str> = required.iter().filter_map(|value| value.as_str()).collect();
    for field in ["provider", "mode", "display_name", "base_url"] {
        assert!(required.contains(&field), "the schema must require {field}");
    }
    assert!(
        !required.contains(&"read_credential"),
        "read_credential defaults to keep, so it is not required"
    );
    for forbidden in [
        "credential_secret_key",
        "webhook_secret_key",
        "app_private_key",
        "installation_id",
    ] {
        assert!(
            !properties.contains_key(forbidden),
            "create schema must not carry {forbidden}"
        );
    }
    let patch_field = properties
        .get("read_credential")
        .expect("read_credential property");
    assert!(!patch_field.is_null(), "the patch field must be defined");
    let schema_text = serde_json::to_string(&serde_json::to_value(&schema).expect("schema value"))
        .expect("schema text");
    for op in ["keep", "set", "clear"] {
        assert!(
            schema_text.contains(op),
            "the create schema must carry the {op} patch op"
        );
    }
    assert_eq!(
        properties
            .get("display_name")
            .and_then(|field| field.get("maxLength"))
            .and_then(|max| max.as_u64()),
        Some(GIT_CONNECTION_DISPLAY_NAME_MAX_CHARS as u64)
    );
    assert_eq!(
        properties
            .get("base_url")
            .and_then(|field| field.get("maxLength"))
            .and_then(|max| max.as_u64()),
        Some(GIT_CONNECTION_BASE_URL_MAX_CHARS as u64)
    );
}
