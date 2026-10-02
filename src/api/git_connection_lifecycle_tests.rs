//! Focused tests for the Maintainer-gated connection disable (issue #681 phase
//! 5F).
//!
//! The route's decisions are pure — the role floor, the bounded not-found
//! mapping, and the safe projection — so the first half builds the stored types in
//! memory and never touches a database, a network, or a secret. The second half
//! reuses the 4B3 disposable-database shape to prove the two things only storage
//! can show: a repeated disable keeps the first timestamp, and an unowned scope
//! reaches neither the read nor the update. Only synthetic keys, statuses, and the
//! disabled flag are reported; no secret value, key, ciphertext, or reference is
//! read or printed here.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use sqlx::Row;

use crate::contracts::sources::{GitConnectionMode, GitProviderKind};
use crate::contracts::{GroupKind, MembershipRole, Visibility};
use crate::db::{
    Database, GitGroupOwnership, NewGitProviderConnection, StoredGitProviderConnection,
};
use crate::domain::GroupRecord;

use super::super::group_access::require_group_role;
use super::{connection_not_found, disabled_by_update};

/// The route module as compiled, so the wiring below is asserted against the real
/// handler rather than against a copy of it.
const ROUTE_SOURCE: &str = include_str!("git_connection_lifecycle.rs");

/// A synthetic key; nothing here is a credential of any deployment.
const CONNECTION_KEY: &str = "github-app";

/// A group id no seeded group ever uses, so a lookup under it matches no row.
///
/// This is the 4B3 convention: the "another group" case is a scope that owns
/// nothing, not a second seeded group — seeding a second group with its own
/// same-key connection would make the read legitimately succeed and would test the
/// fixture instead of the group scoping.
const UNOWNED_GROUP: i64 = 9_999_999;

/// A fixed instant, so the in-memory builders are comparable.
fn timestamp() -> DateTime<Utc> {
    "2026-10-02T00:00:00Z".parse().expect("timestamp")
}

/// The UTF-8 body of a bounded response, for comparing two shapes byte for byte.
async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the bounded body");
    String::from_utf8(bytes.to_vec()).expect("a UTF-8 body")
}

/// A private group with the membership role the Maintainer floor checks.
fn group(current_role: Option<MembershipRole>) -> GroupRecord {
    GroupRecord {
        id: 42,
        parent_group_id: None,
        group_path: "acme/team-platform".to_string(),
        parent_group_path: Some("acme".to_string()),
        group_key: "team-platform".to_string(),
        name: "Team Platform".to_string(),
        visibility: Visibility::Private,
        kind: GroupKind::Shared,
        owner_user_id: None,
        created_at: timestamp(),
        updated_at: timestamp(),
        current_role,
    }
}

/// One stored connection, optionally already disabled.
///
/// The secret fields hold *references* only, exactly as a stored row does: that is
/// what makes the projection test meaningful, because a reference must not reach
/// the response either.
fn connection(disabled: bool) -> StoredGitProviderConnection {
    StoredGitProviderConnection {
        group: GitGroupOwnership {
            group_id: 42,
            group_key: "team-platform".to_string(),
            group_path: "acme/team-platform".to_string(),
            visibility: Visibility::Private,
        },
        connection_key: CONNECTION_KEY.to_string(),
        provider: GitProviderKind::GitHub,
        mode: GitConnectionMode::Token,
        display_name: "GitHub App".to_string(),
        base_url: "https://api.github.com".to_string(),
        credential_secret_key: Some("synthetic-credential-reference".to_string()),
        webhook_secret_key: Some("synthetic-webhook-reference".to_string()),
        app_private_key_secret_key: Some("synthetic-app-key-reference".to_string()),
        disabled_at: disabled.then(timestamp),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

#[test]
fn the_disable_route_shares_the_maintainer_floor_of_the_create_route() {
    for role in [MembershipRole::Maintainer, MembershipRole::Owner] {
        require_group_role(&group(Some(role)), MembershipRole::Maintainer)
            .unwrap_or_else(|error| panic!("{role:?} may disable a connection: {error}"));
    }
    let error = require_group_role(
        &group(Some(MembershipRole::Viewer)),
        MembershipRole::Maintainer,
    )
    .expect_err("a read-only role must not disable a connection")
    .to_string();
    assert!(
        error.contains("insufficient permissions"),
        "a Viewer is refused with the shared forbidden rule: {error}"
    );
    assert!(
        require_group_role(&group(None), MembershipRole::Maintainer).is_err(),
        "a non-member is refused before any read or write"
    );
}

/// The handler must resolve the group through the shared Maintainer helper before
/// it reaches the update. Pinned against the route source, following the 4B3
/// pattern: the ordering *is* the property, the helper's own behaviour is covered
/// by the role-floor test above, and a full router harness is out of scope.
#[test]
fn the_route_checks_the_maintainer_group_before_it_disables() {
    assert!(
        ROUTE_SOURCE.contains("group_access_error_response"),
        "a refused role must answer with the shared group-error mapper"
    );
    // The three call sites below are proven by the search itself; it starts at the
    // handler body, so an import of the helper cannot satisfy it.
    let body = &ROUTE_SOURCE[ROUTE_SOURCE
        .find("pub(crate) async fn disable_git_connection")
        .expect("the route handler")..];
    let role_check = body
        .find("maintainer_group(&state")
        .expect("the handler resolves the group through the shared helper");
    let update = body
        .find("disable_git_provider_connection")
        .expect("the handler calls the existing group-scoped update");
    let read_back = body
        .find("get_git_provider_connection")
        .expect("the handler reads the connection back to project it");
    assert!(
        role_check < update,
        "a Viewer or non-member must be refused before anything is disabled"
    );
    assert!(
        update < read_back,
        "the projection is read back after the update, never before it"
    );
    // The preflight the plan forbids must not come back: the only connection read
    // in the handler is the one that builds the response.
    assert_eq!(
        body.matches("get_git_provider_connection").count(),
        1,
        "the handler reads the connection exactly once, after the update"
    );
}

#[tokio::test]
async fn an_unmatched_update_answers_exactly_as_an_unknown_connection() {
    // The handler decides from the update's own result through `disabled_by_update`,
    // so this exercises the arm the route actually takes: a drift back to a
    // distinguishable "no such connection in this group" shape fails here.
    let unknown = connection_not_found();
    let unknown_status = unknown.status();
    let unknown_body = response_body(unknown).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert!(
        unknown_body.contains("unknown git provider connection"),
        "the shared shape carries the bounded message: {unknown_body}"
    );

    // `false` is what a foreign connection and a key that names nothing both report,
    // and it is the only input that carries the bounded `404`.
    let unmatched = *disabled_by_update(Ok(false)).expect_err("no matched row is not a disable");
    assert_eq!(
        unmatched.status(),
        unknown_status,
        "the status must be shared"
    );
    assert_eq!(
        response_body(unmatched).await,
        unknown_body,
        "an unmatched update must not be distinguishable by body either"
    );
    for detail in [CONNECTION_KEY, "team-platform", "api.github.com"] {
        assert!(
            !unknown_body.contains(detail),
            "the 404 must not echo {detail}: {unknown_body}"
        );
    }
    // A matched update — the first disable and every repeat of it — is the same
    // success rather than a conflict, because the stored `COALESCE` reports a match
    // again instead of refusing a second disable.
    for attempt in 1..=2 {
        assert!(
            disabled_by_update(Ok(true)).is_ok(),
            "attempt {attempt}: a matched update continues to the projection"
        );
    }
}

#[test]
fn the_disabled_projection_reports_presence_and_never_a_secret() {
    let contract = connection(false).to_contract();
    assert!(!contract.disabled, "an enabled row projects as enabled");
    assert!(contract.has_read_credential && contract.has_webhook_secret);
    assert!(
        connection(true).to_contract().disabled,
        "a disabled row projects as disabled"
    );

    let serialized = serde_json::to_string(&contract).expect("the projection serializes");
    let encoded = serde_json::to_value(&contract).expect("the projection serializes");
    let mut keys: Vec<&str> = encoded
        .as_object()
        .expect("a projection object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys.join(" "),
        "base_url connection_key created_at disabled display_name group_key group_path \
         has_read_credential has_webhook_secret mode provider updated_at visibility",
        "the projection reports which secrets are configured and nothing more"
    );
    for forbidden in [
        "credential_secret_key",
        "webhook_secret_key",
        "app_private_key",
        "synthetic-credential-reference",
        "synthetic-webhook-reference",
        "synthetic-app-key-reference",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the disabled projection must not carry {forbidden}: {serialized}"
        );
    }
}

/// One seeded group and one stored connection, in the 4B3 disposable shape: a
/// fresh group key per run, and no secret material at all.
async fn fixture(db: &Database, label: &str) -> i64 {
    let group_key = format!("git-connection-{label}-{}", Uuid::new_v4());
    let group_id = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, 'Git Connection Disable Test', 'private', 'shared', $2) RETURNING id",
    )
    .bind(&group_key)
    .bind(format!("/{group_key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id");
    db.insert_git_provider_connection(
        group_id,
        &NewGitProviderConnection {
            connection_key: CONNECTION_KEY.to_string(),
            provider: GitProviderKind::GitHub,
            mode: GitConnectionMode::Token,
            display_name: "GitHub App".to_string(),
            base_url: "https://api.github.com".to_string(),
            credential_secret_key: None,
            webhook_secret_key: None,
        },
    )
    .await
    .expect("seed a connection");
    group_id
}

/// The scratch database, or `None` when the gated round trips must skip.
async fn scratch() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

async fn cleanup(db: &Database, group_ids: &[i64]) {
    for group_id in group_ids {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean up a test group");
    }
}

#[tokio::test]
async fn disabling_twice_keeps_the_first_timestamp_and_answers_twice() {
    let Some(db) = scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the disable round trip");
        return;
    };
    let group_id = fixture(&db, "disable").await;
    let before = db
        .get_git_provider_connection(group_id, CONNECTION_KEY)
        .await
        .expect("read the connection")
        .expect("the connection is stored");
    assert!(before.disabled_at.is_none(), "a new row is enabled");

    // The route's exact sequence, once per attempt: guard, update, read back.
    let mut first_disable = None;
    for attempt in 1..=2 {
        assert!(
            db.get_git_provider_connection(group_id, CONNECTION_KEY)
                .await
                .expect("read the owned connection")
                .is_some(),
            "attempt {attempt}: the owner's connection is there to disable"
        );
        assert!(
            db.disable_git_provider_connection(group_id, CONNECTION_KEY)
                .await
                .expect("disable the connection"),
            "attempt {attempt}: a repeat is still a successful update"
        );
        let stored = db
            .get_git_provider_connection(group_id, CONNECTION_KEY)
            .await
            .expect("read the connection back")
            .expect("the connection is stored");
        assert!(
            stored.to_contract().disabled,
            "attempt {attempt}: the projection is disabled"
        );
        let disabled_at = stored.disabled_at.expect("the row is disabled");
        match first_disable {
            None => first_disable = Some(disabled_at),
            Some(first) => assert_eq!(
                disabled_at, first,
                "attempt {attempt}: the first disable timestamp survives the repeat"
            ),
        }
    }
    assert!(first_disable.is_some(), "both attempts disabled the row");
    let serialized = serde_json::to_string(
        &db.get_git_provider_connection(group_id, CONNECTION_KEY)
            .await
            .expect("read the connection")
            .expect("the connection is stored")
            .to_contract(),
    )
    .expect("the projection serializes");
    for forbidden in ["secret_key", "app_private_key", "credential_secret_key"] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }

    cleanup(&db, &[group_id]).await;
}

#[tokio::test]
async fn an_unowned_scope_reaches_neither_the_read_nor_the_update() {
    let Some(db) = scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the disable round trip");
        return;
    };
    let group_id = fixture(&db, "disable-owner").await;

    // The owner's key read under a scope that owns nothing is absent and matches
    // no update, so the 404 is the same one an unknown key gets and nothing is
    // disabled.
    assert!(
        db.get_git_provider_connection(UNOWNED_GROUP, CONNECTION_KEY)
            .await
            .expect("read the unowned scope")
            .is_none(),
        "a scope that owns nothing reads no connection"
    );
    assert!(
        !db.disable_git_provider_connection(UNOWNED_GROUP, CONNECTION_KEY)
            .await
            .expect("attempt a disable from an unowned scope"),
        "an unowned scope matches no row"
    );
    let owned = db
        .get_git_provider_connection(group_id, CONNECTION_KEY)
        .await
        .expect("read the owning group's connection")
        .expect("the connection is stored");
    assert!(
        owned.disabled_at.is_none() && !owned.to_contract().disabled,
        "an attempt from another scope must not disable the owner's connection"
    );

    cleanup(&db, &[group_id]).await;
}
