//! Focused tests for Maintainer Git provider connection creation (issue #681
//! phase 4B3).
//!
//! The pure cases pin the decision and mapping logic in front of persistence:
//! an error that carries no unique violation is not a conflict, and a duplicate
//! is reported without a secret-bearing value. The database cases run against a
//! disposable migrated schema named by `CONTEXT69_TEST_DATABASE_URL` and cover
//! create-only metadata persistence, `Keep`/`Set` credential behavior, the
//! seal-before-insert ordering, and the group confinement of the pre-check, the
//! insert, and the sealed reference. They report statuses and boolean facts
//! only and never print a raw value or a derived secret-store key.

use super::{CreateConnectionFailure, create_group_connection, is_unique_violation};

use crate::{
    contracts::sources::{
        GitConnectionMode, GitProviderConnectionRequest, GitProviderKind, GitReadCredentialPatch,
    },
    db::{Database, NewGitProviderConnection},
    services::{
        git_secrets::{GitSecretReader, GitSecretSlot, GitSecretTarget},
        secret_store::SecretStore,
    },
};
use sqlx::Row;
use uuid::Uuid;

/// The route source, so a wiring assertion reads the same file the compiler
/// builds without standing up a full application/router harness.
const ROUTE_SOURCE: &str = include_str!("git_connection_mutations.rs");

const CONNECTION_KEY: &str = "github-app";
/// A group id no seeded group ever uses, so a lookup matches no row.
const UNOWNED_GROUP: i64 = 9_999_999;

/// A fresh synthetic credential for one round trip.
///
/// It is generated per test rather than written as a literal, so no
/// token-shaped value is embedded in the suite, and it is never placed in an
/// assertion message or a panic payload.
fn synthetic_credential() -> String {
    format!("synthetic-{}", Uuid::new_v4())
}

fn request(
    mode: GitConnectionMode,
    credential: GitReadCredentialPatch,
) -> GitProviderConnectionRequest {
    GitProviderConnectionRequest {
        provider: GitProviderKind::GitHub,
        mode,
        display_name: "GitHub PAT".to_string(),
        base_url: "https://api.github.com".to_string(),
        read_credential: credential,
    }
}

fn keep() -> GitReadCredentialPatch {
    GitReadCredentialPatch::Keep
}

#[test]
fn only_a_postgres_unique_violation_is_mapped_to_a_conflict() {
    assert!(!is_unique_violation(&anyhow::anyhow!(
        "plain storage failure: connection refused"
    )));
    let wrapped = anyhow::anyhow!("plain storage failure").context("outer context");
    assert!(!is_unique_violation(&wrapped));
}

/// A malformed path key is refused locally with the existing bounded
/// invalid-argument shape before any group or database I/O, and the response
/// never echoes the submitted key.
#[tokio::test]
async fn a_rejected_key_uses_the_bounded_invalid_argument_response() {
    use axum::http::StatusCode;

    use super::invalid_request;
    use crate::contracts::sources::validate_git_connection_key;

    for key in ["", "  ", "bad key", "a/b", "..", "."] {
        let rejection = validate_git_connection_key(key).expect_err("unsafe key");
        let response = invalid_request(rejection.as_str());
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the bounded body");
        let body = String::from_utf8(bytes.to_vec()).expect("utf8 body");
        assert!(
            body.contains(rejection.as_str()),
            "the bounded reason must be present"
        );
        assert!(
            key.is_empty() || !body.contains(key),
            "the response must not echo the submitted key"
        );
    }
}

/// The route resolves the group and enforces Maintainer access through the
/// shared group-access helpers, and its group-error branch is the shared
/// bounded response — pinned by reading the route source, so the assertion
/// cannot drift from the compiled handler without a full router harness.
#[test]
fn the_route_wires_group_lookup_role_check_and_bounded_group_error() {
    for seam in [
        "group_for_user",
        "require_group_role",
        "MembershipRole::Maintainer",
        "group_access_error_response",
    ] {
        assert!(
            ROUTE_SOURCE.contains(seam),
            "create_git_connection must wire {seam}"
        );
    }
    // A malformed path key is refused before the group lookup, so it never
    // reaches group or database I/O. The search starts at the handler body so
    // the import of the validator cannot satisfy it.
    let body = &ROUTE_SOURCE[ROUTE_SOURCE
        .find("pub(crate) async fn create_git_connection")
        .expect("the route handler")..];
    let key_check = body
        .find("validate_git_connection_key")
        .expect("the route validates the path key");
    let group_lookup = body
        .find("maintainer_group")
        .expect("the route resolves the group through the shared helper");
    assert!(
        key_check < group_lookup,
        "the path key must be validated before the group lookup"
    );
}

/// The create path takes the per-key transaction lock before it seals and
/// inserts, so two same-key creates cannot both seal the one deterministic
/// encrypted row before either inserts. Pinned against the route source because
/// the ordering is the repair and a full router harness is out of scope.
#[test]
fn the_create_path_holds_the_key_lock_across_seal_and_insert() {
    let lock = ROUTE_SOURCE
        .find("begin_git_connection_creation")
        .expect("the create path takes the per-key creation lock");
    let seal = ROUTE_SOURCE
        .find("seal_read_credential")
        .expect("the create path seals the read credential");
    let insert = ROUTE_SOURCE
        .find("creation.insert")
        .expect("the create path runs the create-only insert");
    let commit = ROUTE_SOURCE
        .find("creation.commit")
        .expect("the create path commits to release the lock");
    assert!(
        lock < seal && seal < insert && insert < commit,
        "the lock must be held across seal and insert and released by commit"
    );
}

/// A foreign and an unknown group share the shared bounded not-found shape, so
/// the route's group-error branch reveals nothing about which groups exist and
/// never echoes the submitted path.
#[tokio::test]
async fn a_missing_group_maps_to_the_bounded_non_disclosure_shape() {
    use axum::http::StatusCode;

    use crate::domain_errors::DomainError;

    // Exactly the error `group_for_user` produces for both an unknown path and
    // a group the caller cannot see; the route feeds it to the shared mapper.
    let response = crate::api::error_mapping::group_access_error_response(
        DomainError::not_found("unknown group").into(),
    );
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the bounded body");
    let body = String::from_utf8(bytes.to_vec()).expect("utf8 body");
    assert!(
        body.contains("unknown group"),
        "the bounded group not-found message must be present"
    );
    assert!(
        !body.contains("/v1/") && !body.contains("group_path"),
        "the group-error shape must not echo routing detail"
    );
}

/// One seeded group and a keyed store over the scratch pool. Skipped unless
/// `CONTEXT69_TEST_DATABASE_URL` names a migrated disposable database; each run
/// uses a fresh group key and master key, so rows never collide.
async fn fixture() -> Option<(Database, SecretStore, i64)> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db, "create").await;
    // A per-run key, so nothing here is a credential of any deployment.
    let master_key = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        Uuid::new_v4().into_bytes().repeat(2),
    );
    let store = SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        Some(master_key.as_str()),
        1,
    )
    .expect("a store builds from any master key");
    Some((db, store, group_id))
}

fn target(group_id: i64) -> GitSecretTarget {
    GitSecretTarget::Connection {
        group_id,
        connection_key: CONNECTION_KEY.to_string(),
    }
}

async fn stored_credential(store: &SecretStore, group_id: i64) -> Option<Vec<u8>> {
    GitSecretReader::new(store.clone())
        .read(&target(group_id), GitSecretSlot::Credential)
        .await
        .expect("open the credential")
        .map(|value| value.expose().to_vec())
}

async fn seed_group(db: &Database, label: &str) -> i64 {
    let group_key = format!("git-connection-{label}-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, 'Git Connection Create Test', 'private', 'shared', $2) RETURNING id",
    )
    .bind(&group_key)
    .bind(format!("/{group_key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id")
}

#[tokio::test]
async fn keep_creates_an_uncredentialed_enabled_connection() {
    let Some((db, store, group_id)) = fixture().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the create round trip");
        return;
    };
    let stored = create_group_connection(
        &db,
        &store,
        group_id,
        CONNECTION_KEY,
        &request(GitConnectionMode::Public, keep()),
    )
    .await
    .expect("create a public connection");

    assert_eq!(stored.connection_key, CONNECTION_KEY);
    assert_eq!(stored.provider, GitProviderKind::GitHub);
    assert_eq!(stored.mode, GitConnectionMode::Public);
    assert_eq!(stored.display_name, "GitHub PAT");
    assert_eq!(stored.base_url, "https://api.github.com");
    assert!(stored.credential_secret_key.is_none());
    assert!(stored.webhook_secret_key.is_none());
    assert!(
        stored.disabled_at.is_none(),
        "a new row is enabled, never disabled"
    );
    let contract = stored.to_contract();
    assert!(!contract.has_read_credential);
    assert!(!contract.has_webhook_secret);
    assert!(!contract.disabled);
    assert!(
        stored_credential(&store, group_id).await.is_none(),
        "keep must not seal any value"
    );
}

#[tokio::test]
async fn set_seals_the_credential_and_projects_presence_only() {
    let Some((db, store, group_id)) = fixture().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the create round trip");
        return;
    };
    let credential = synthetic_credential();
    let stored = create_group_connection(
        &db,
        &store,
        group_id,
        CONNECTION_KEY,
        &request(
            GitConnectionMode::Token,
            GitReadCredentialPatch::Set(credential.clone()),
        ),
    )
    .await
    .expect("create a token connection");

    assert_eq!(stored.mode, GitConnectionMode::Token);
    assert!(
        stored.credential_secret_key.is_some(),
        "set must attach a credential reference"
    );
    let contract = stored.to_contract();
    assert!(contract.has_read_credential);
    let serialized = serde_json::to_string(&contract).expect("contract serializes");
    assert!(
        !serialized.contains(&credential),
        "the projection must not carry the sealed credential value"
    );
    for forbidden in [
        "credential_secret_key",
        "webhook_secret_key",
        "app_private_key",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the projection must not carry {forbidden}"
        );
    }
    assert!(
        stored_credential(&store, group_id).await.as_deref() == Some(credential.as_bytes()),
        "the sealed value round-trips through the store"
    );
}

#[tokio::test]
async fn a_set_credential_rides_the_insert_and_no_follow_up_update_runs() {
    let Some((db, store, group_id)) = fixture().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ordering case");
        return;
    };
    let credential = synthetic_credential();
    let stored = create_group_connection(
        &db,
        &store,
        group_id,
        CONNECTION_KEY,
        &request(
            GitConnectionMode::Token,
            GitReadCredentialPatch::Set(credential.clone()),
        ),
    )
    .await
    .expect("create a token connection");

    // The row returned by the insert already carries the reference, and it is
    // the deterministic name the shared reader derives, so the value
    // round-trips without any repointing step.
    assert!(
        stored.credential_secret_key.is_some(),
        "the insert must carry the reference, not insert then repoint it"
    );
    assert!(
        stored_credential(&store, group_id).await.as_deref() == Some(credential.as_bytes()),
        "the sealed value round-trips through the store"
    );
    // A seal-before-insert is one statement: `created_at` and `updated_at` both
    // come from that statement's `now()`. The repointing writer, which attaches
    // a secret to an existing row, would have run a second UPDATE and left
    // `updated_at` strictly later than `created_at`.
    assert_eq!(
        stored.created_at, stored.updated_at,
        "a create must insert the reference in the create statement, not repoint after"
    );
    let serialized = serde_json::to_string(&stored.to_contract()).expect("contract serializes");
    assert!(
        !serialized.contains(&credential),
        "the projection must not carry the sealed credential value"
    );
    let reference = stored
        .credential_secret_key
        .clone()
        .expect("the insert carries a reference");
    assert!(
        !serialized.contains(&reference),
        "the projection must not carry the secret-store reference"
    );
    for forbidden in [
        "credential_secret_key",
        "webhook_secret_key",
        "app_private_key",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the projection must not carry {forbidden}"
        );
    }
}

#[tokio::test]
async fn keep_creates_an_enabled_installation_connection_without_a_secret() {
    let Some((db, store, group_id)) = fixture().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the installation case");
        return;
    };
    let stored = create_group_connection(
        &db,
        &store,
        group_id,
        CONNECTION_KEY,
        &request(GitConnectionMode::Installation, keep()),
    )
    .await
    .expect("create an installation connection");

    assert_eq!(stored.mode, GitConnectionMode::Installation);
    assert_eq!(stored.provider, GitProviderKind::GitHub);
    assert_eq!(stored.connection_key, CONNECTION_KEY);
    assert_eq!(stored.display_name, "GitHub PAT");
    assert_eq!(stored.base_url, "https://api.github.com");
    assert!(
        stored.disabled_at.is_none(),
        "an installation connection is created enabled"
    );
    assert!(stored.credential_secret_key.is_none());
    assert!(stored.webhook_secret_key.is_none());
    assert!(
        stored.app_private_key_secret_key.is_none(),
        "the App-key reference stays deferred; a create never sets it"
    );
    let contract = stored.to_contract();
    assert!(!contract.has_read_credential);
    assert!(!contract.has_webhook_secret);
    assert!(!contract.disabled);
    assert_eq!(contract.mode, GitConnectionMode::Installation);
    assert!(
        stored_credential(&store, group_id).await.is_none(),
        "keep must not seal any value"
    );
}

#[tokio::test]
async fn the_insert_surface_reports_a_real_unique_violation_error() {
    let Some((db, _store, group_id)) = fixture().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the unique-violation case");
        return;
    };
    let new = NewGitProviderConnection {
        connection_key: CONNECTION_KEY.to_string(),
        provider: GitProviderKind::GitHub,
        mode: GitConnectionMode::Public,
        display_name: "GitHub PAT".to_string(),
        base_url: "https://api.github.com".to_string(),
        credential_secret_key: None,
        webhook_secret_key: None,
    };
    db.insert_git_provider_connection(group_id, &new)
        .await
        .expect("insert the first row");

    // Bypass the pre-check and reach the statement directly, so the error the
    // create path's race mapping consumes is the real PostgreSQL one. The
    // precheck behavior keeps its own test below.
    let error = db
        .insert_git_provider_connection(group_id, &new)
        .await
        .expect_err("a duplicate insert raises a unique violation");
    assert!(
        is_unique_violation(&error),
        "the create path maps this exact error to a bounded conflict"
    );
    assert!(
        error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::Database(database_error))
                    if database_error.code().as_deref() == Some("23505")
            )
        }),
        "the violation carries the documented SQLSTATE"
    );
    // The statement is create-only: the failed insert left the first row alone.
    let survivor = db
        .get_git_provider_connection(group_id, CONNECTION_KEY)
        .await
        .expect("read the connection")
        .expect("the first row survives");
    assert!(survivor.disabled_at.is_none());
    assert!(survivor.credential_secret_key.is_none());
}

#[tokio::test]
async fn a_duplicate_key_is_refused_before_any_seal() {
    let Some((db, store, group_id)) = fixture().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the duplicate case");
        return;
    };
    create_group_connection(
        &db,
        &store,
        group_id,
        CONNECTION_KEY,
        &request(GitConnectionMode::Public, keep()),
    )
    .await
    .expect("create the first connection");

    let failure = create_group_connection(
        &db,
        &store,
        group_id,
        CONNECTION_KEY,
        &request(
            GitConnectionMode::Token,
            GitReadCredentialPatch::Set(synthetic_credential()),
        ),
    )
    .await
    .expect_err("a duplicate key is refused");
    assert!(matches!(failure, CreateConnectionFailure::AlreadyExists));

    let survivor = db
        .get_git_provider_connection(group_id, CONNECTION_KEY)
        .await
        .expect("read the connection")
        .expect("the first row survives");
    assert_eq!(
        survivor.mode,
        GitConnectionMode::Public,
        "a refused duplicate must not overwrite the row"
    );
    assert!(
        survivor.credential_secret_key.is_none(),
        "the first row keeps its own (absent) credential"
    );
    assert!(
        stored_credential(&store, group_id).await.is_none(),
        "a duplicate refused before sealing must leave no sealed value"
    );
}

#[tokio::test]
async fn a_create_can_only_reach_its_own_group() {
    let Some((db, store, group_id)) = fixture().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the confinement case");
        return;
    };
    let other_group = seed_group(&db, "other").await;
    let credential = synthetic_credential();
    let other_credential = synthetic_credential();
    let first = create_group_connection(
        &db,
        &store,
        group_id,
        CONNECTION_KEY,
        &request(
            GitConnectionMode::Token,
            GitReadCredentialPatch::Set(credential.clone()),
        ),
    )
    .await
    .expect("create in the owning group");
    assert!(first.credential_secret_key.is_some());

    // Another group may use the same key with its own credential; the two rows
    // and their encrypted values stay independent, and neither group can read
    // the other's connection.
    let second = create_group_connection(
        &db,
        &store,
        other_group,
        CONNECTION_KEY,
        &request(
            GitConnectionMode::Token,
            GitReadCredentialPatch::Set(other_credential.clone()),
        ),
    )
    .await
    .expect("the same key is free in another group");
    assert!(second.credential_secret_key.is_some());
    assert_ne!(
        first.credential_secret_key, second.credential_secret_key,
        "the group id is part of the derived name, so two groups never share one"
    );

    assert!(
        db.get_git_provider_connection(UNOWNED_GROUP, CONNECTION_KEY)
            .await
            .expect("read the unowned group")
            .is_none(),
        "an unknown group reads nothing"
    );
    let owner = db
        .get_git_provider_connection(group_id, CONNECTION_KEY)
        .await
        .expect("read the owner")
        .expect("the owner's row remains");
    assert_eq!(
        owner.credential_secret_key, first.credential_secret_key,
        "another group's create must not move the owner's reference"
    );
    assert!(
        stored_credential(&store, group_id).await.as_deref() == Some(credential.as_bytes()),
        "the owner's sealed value round-trips through the store"
    );
    assert!(
        stored_credential(&store, other_group).await.as_deref()
            == Some(other_credential.as_bytes()),
        "the other group's sealed value round-trips through the store"
    );
}

/// The create transaction serializes concurrent same-key creates, so exactly
/// one wins and the stored encrypted row keeps the winner's credential.
///
/// Without the lock both requests pass the pre-check, both seal the same
/// deterministic name, and the store's write-else-rotate would leave whichever
/// sealed last — the two futures interleave at the create awaits even on the
/// default single-threaded runtime. The rounds run with fresh keys so a rare
/// schedule cannot hide the property.
#[tokio::test]
async fn concurrent_same_key_creates_serialize_to_one_winner_with_its_credential() {
    for _ in 0..3 {
        let Some((db, store, group_id)) = fixture().await else {
            eprintln!(
                "CONTEXT69_TEST_DATABASE_URL is not set; skipping the concurrent create case"
            );
            return;
        };
        let key = format!("github-app-{}", Uuid::new_v4());
        let first = synthetic_credential();
        let second = synthetic_credential();
        let left_request = request(
            GitConnectionMode::Token,
            GitReadCredentialPatch::Set(first.clone()),
        );
        let right_request = request(
            GitConnectionMode::Token,
            GitReadCredentialPatch::Set(second.clone()),
        );
        let (left, right) = tokio::join!(
            create_group_connection(&db, &store, group_id, &key, &left_request),
            create_group_connection(&db, &store, group_id, &key, &right_request),
        );
        let (winner, winner_credential) = match (left, right) {
            (Ok(stored), Err(CreateConnectionFailure::AlreadyExists)) => (stored, first.clone()),
            (Err(CreateConnectionFailure::AlreadyExists), Ok(stored)) => (stored, second.clone()),
            _ => panic!("exactly one concurrent create must win the key"),
        };
        assert!(
            winner.credential_secret_key.is_some(),
            "the winner carries the deterministic credential reference"
        );
        let target = GitSecretTarget::Connection {
            group_id,
            connection_key: key.clone(),
        };
        let sealed = GitSecretReader::new(store.clone())
            .read(&target, GitSecretSlot::Credential)
            .await
            .expect("open the credential")
            .expect("the winner sealed a credential")
            .expose()
            .to_vec();
        assert!(
            sealed == winner_credential.as_bytes(),
            "the row a 201 advertises must hold the winner's own credential"
        );
        let loser = if winner_credential == first {
            &second
        } else {
            &first
        };
        assert!(
            !(sealed == loser.as_bytes()),
            "the loser must never rotate the winner's encrypted row"
        );
    }
}
