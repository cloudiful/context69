//! Provider-neutral Git source and connection contracts (issue #681 phases 2
//! and 3B1).
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
    GitActiveGeneration, GitCommitCheckpoint, GitConnectionMode, GitGenerationStatus,
    GitIndexProfile, GitIndexStatus, GitProviderConnection, GitProviderKind, GitRefreshPolicy,
    GitRepositoryGeneration, GitRepositoryRegistrationRequest, GitRepositorySource,
    GitVersionPolicy, GitWebhookDelivery, GitWebhookDeliveryStatus, GitWebhookOwnership,
    GitWebhookRegistration,
};
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
