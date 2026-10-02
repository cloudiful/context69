//! Focused tests for the Maintainer-gated connection enable (issue #681 phase 5G).
//!
//! Like the disable arm's suite, the decisions are pure — the wiring order, the
//! bounded not-found mapping, and the projection — so the first half never touches
//! a database, a network, or a secret. The second half reuses the disable suite's
//! disposable-database shape to prove what only storage can show: an enable on an
//! already-enabled row, a disable/enable cycle, and group confinement. Only
//! synthetic keys and statuses are reported; no secret value is ever read here.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use sqlx::Row;

use crate::api::group_access::require_group_role;
use crate::contracts::sources::{GitConnectionMode, GitProviderKind};
use crate::contracts::{GroupKind, MembershipRole, Visibility};
use crate::db::{Database, NewGitProviderConnection, StoredGitProviderConnection};
use crate::domain::GroupRecord;

use super::{connection_not_found, enabled_by_update};

/// The compiled route module, so the wiring below is asserted against the real
/// handler rather than a copy of it.
const ROUTE_SOURCE: &str = include_str!("git_connection_lifecycle.rs");

/// A synthetic key; nothing here is a credential of any deployment.
const CONNECTION_KEY: &str = "github-app";

/// The one value every synthetic store row carries. It is a fixed marker, not a
/// credential, a ciphertext, or anything derived from a real secret.
const SYNTHETIC_STORE_VALUE: &[u8] = b"synthetic-not-a-credential";

/// A group id no seeded group ever uses, so a lookup under it matches no row.
const UNOWNED_GROUP: i64 = 9_999_999;

/// A fixed instant, so the in-memory builders are comparable.
fn timestamp() -> DateTime<Utc> {
    "2026-10-02T00:00:00Z".parse().expect("timestamp")
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

/// The UTF-8 body of a bounded response, for comparing two shapes byte for byte.
async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the bounded body");
    String::from_utf8(bytes.to_vec()).expect("a UTF-8 body")
}

#[test]
fn the_enable_route_shares_the_maintainer_floor_of_the_other_lifecycle_paths() {
    // The enable path reuses the one shared role helper, so a Viewer or a
    // non-member is refused with the same rule the create and disable paths use.
    for role in [MembershipRole::Maintainer, MembershipRole::Owner] {
        require_group_role(&group(Some(role)), MembershipRole::Maintainer)
            .unwrap_or_else(|error| panic!("{role:?} may enable a connection: {error}"));
    }
    assert!(
        require_group_role(
            &group(Some(MembershipRole::Viewer)),
            MembershipRole::Maintainer
        )
        .is_err(),
        "a read-only role must not enable a connection"
    );
    assert!(
        require_group_role(&group(None), MembershipRole::Maintainer).is_err(),
        "a non-member is refused before any read or write"
    );
    assert!(
        ROUTE_SOURCE.contains("maintainer_group(&state, session.user.id, &group_path)"),
        "the enable handler must resolve the group through the shared helper"
    );
}

#[test]
fn the_route_checks_the_maintainer_group_before_it_enables() {
    assert!(
        ROUTE_SOURCE.contains("group_access_error_response"),
        "a refused role must answer with the shared group-error mapper"
    );
    // The three call sites below are proven by the search itself, and the body is
    // bounded to this handler so an import or a neighbouring handler cannot satisfy
    // them.
    let start = ROUTE_SOURCE
        .find("pub(crate) async fn enable_git_connection")
        .expect("the enable handler");
    let rest = &ROUTE_SOURCE[start..];
    let end = rest
        .find("\npub(crate) async fn ")
        .map(|offset| offset + 1)
        .unwrap_or(rest.len());
    let body = &rest[..end];
    let role_check = body
        .find("maintainer_group(&state")
        .expect("the handler resolves the group through the shared helper");
    let update = body
        .find("enable_git_provider_connection")
        .expect("the handler calls the existing group-scoped update");
    let read_back = body
        .find("get_git_provider_connection")
        .expect("the handler reads the connection back to project it");
    assert!(
        role_check < update,
        "a Viewer or non-member must be refused before anything is enabled"
    );
    assert!(
        update < read_back,
        "the projection is read back after the update, never before it"
    );
    assert_eq!(
        body.matches("get_git_provider_connection").count(),
        1,
        "this handler reads the connection exactly once, after the update, so it cannot \
         grow a preflight"
    );
}

#[tokio::test]
async fn an_unmatched_enable_answers_exactly_as_an_unknown_connection() {
    // The handler decides from the update's own result, so this exercises the arm
    // the route actually takes: a drift back to a distinguishable "no such
    // connection in this group" shape fails here.
    let unknown = connection_not_found();
    let unknown_status = unknown.status();
    let unknown_body = response_body(unknown).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert!(
        unknown_body.contains("unknown git provider connection"),
        "the shared shape carries the bounded message: {unknown_body}"
    );
    let unmatched = *enabled_by_update(Ok(false)).expect_err("no matched row is not an enable");
    assert_eq!(
        unmatched.status(),
        unknown_status,
        "the status must be shared"
    );
    assert_eq!(
        response_body(unmatched).await,
        unknown_body,
        "an unmatched enable must not be distinguishable by body either"
    );
    for detail in [CONNECTION_KEY, "team-platform", "api.github.com"] {
        assert!(
            !unknown_body.contains(detail),
            "the 404 must not echo {detail}: {unknown_body}"
        );
    }
    // A matched update is the same success for an already-enabled connection and for
    // every repeat, because the statement never requires the row to be disabled.
    for attempt in 1..=2 {
        assert!(
            enabled_by_update(Ok(true)).is_ok(),
            "attempt {attempt}: a matched update continues to the projection"
        );
    }
}

/// The three opaque references a lifecycle cycle must leave where it found them.
fn references(row: &StoredGitProviderConnection) -> [Option<String>; 3] {
    [
        row.credential_secret_key.clone(),
        row.webhook_secret_key.clone(),
        row.app_private_key_secret_key.clone(),
    ]
}

/// The stored connection of one group, as the route reads it back.
async fn stored(db: &Database, group_id: i64) -> StoredGitProviderConnection {
    db.get_git_provider_connection(group_id, CONNECTION_KEY)
        .await
        .expect("read the connection")
        .expect("the connection is stored")
}

/// One seeded group and one connection carrying all three synthetic secret
/// references, plus the store rows those references point at.
///
/// A per-run key per slot and one fixed non-credential byte string behind them
/// satisfy the references' foreign keys without this suite sealing or reading a
/// real secret. The App key goes through the existing helper, as the create path
/// does, because the broad upsert has no column for it.
async fn fixture(db: &Database, label: &str) -> (i64, Vec<String>) {
    let group_key = format!("git-connection-{label}-{}", Uuid::new_v4());
    let group_id = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, 'Git Connection Enable Test', 'private', 'shared', $2) RETURNING id",
    )
    .bind(&group_key)
    .bind(format!("/{group_key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id");
    let keys: Vec<String> = ["credential", "webhook", "app-key"]
        .iter()
        .map(|slot| format!("git-connection-{label}-{slot}-{}", Uuid::new_v4()))
        .collect();
    for key in &keys {
        sqlx::query("INSERT INTO context69.internal_secrets (key, value) VALUES ($1, $2)")
            .bind(key)
            .bind(SYNTHETIC_STORE_VALUE)
            .execute(db.pool())
            .await
            .expect("seed a store row the reference points at");
    }
    db.insert_git_provider_connection(
        group_id,
        &NewGitProviderConnection {
            connection_key: CONNECTION_KEY.to_string(),
            provider: GitProviderKind::GitHub,
            mode: GitConnectionMode::Token,
            display_name: "GitHub App".to_string(),
            base_url: "https://api.github.com".to_string(),
            credential_secret_key: Some(keys[0].clone()),
            webhook_secret_key: Some(keys[1].clone()),
        },
    )
    .await
    .expect("seed a connection");
    assert!(
        db.set_git_connection_app_private_key_secret_key(group_id, CONNECTION_KEY, &keys[2])
            .await
            .expect("attach the App private-key reference"),
        "the App key reference is stored alongside the other two"
    );
    (group_id, keys)
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

/// Removes the seeded groups, then the store rows behind their references.
///
/// The group delete cascades the connection rows; every reference to a store row
/// is `ON DELETE SET NULL`, so the rows survive that cascade and are removed
/// explicitly instead of leaking into the next run.
async fn cleanup(db: &Database, groups: &[(i64, Vec<String>)]) {
    for (group_id, keys) in groups {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean up a test group");
        for key in keys {
            sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
                .bind(key)
                .execute(db.pool())
                .await
                .expect("clean up a test store row");
        }
    }
}

#[tokio::test]
async fn a_disable_enable_cycle_preserves_the_stored_references() {
    let Some(db) = scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the enable round trip");
        return;
    };
    let (group_id, keys) = fixture(&db, "enable-cycle").await;
    let before = stored(&db, group_id).await;
    assert!(before.disabled_at.is_none(), "a new row is enabled");
    // Every slot must already hold a present, distinct reference, or the
    // comparisons below would only be proving that absent equals absent.
    let before_references = references(&before);
    for (slot, reference) in before_references.iter().enumerate() {
        assert!(
            reference.is_some() && reference.as_deref() == Some(keys[slot].as_str()),
            "the fixture stores a distinct reference for slot {slot}"
        );
    }

    // An enable on an already-enabled connection is the same successful match, and
    // it keeps the configured-secret presence exactly as it was.
    assert!(
        db.enable_git_provider_connection(group_id, CONNECTION_KEY)
            .await
            .expect("enable an already-enabled connection"),
        "enabling an enabled connection is idempotent"
    );
    let once = stored(&db, group_id).await;
    assert!(once.disabled_at.is_none() && !once.to_contract().disabled);
    assert!(
        references(&once) == before_references,
        "an enable must not move a stored reference"
    );

    // Disable, then enable again: the first transition and the second are both
    // matches, and the presence flags survive the whole cycle unchanged.
    for (label, enabled) in [("disable", false), ("enable", true), ("enable", true)] {
        let matched = if enabled {
            db.enable_git_provider_connection(group_id, CONNECTION_KEY)
                .await
        } else {
            db.disable_git_provider_connection(group_id, CONNECTION_KEY)
                .await
        };
        assert!(matched.expect(label), "{label} must match a row");
        let current = stored(&db, group_id).await;
        let contract = current.to_contract();
        assert_eq!(
            contract.disabled, !enabled,
            "{label} leaves the connection in the expected state"
        );
        assert!(
            contract.has_read_credential && contract.has_webhook_secret,
            "{label} keeps the configured-secret presence"
        );
        assert!(
            references(&current) == before_references,
            "{label} must not move a stored reference"
        );
    }
    // The projection of that row is presence-only: exactly the thirteen safe
    // fields, and none of the three stored references anywhere in it. The frozen
    // 5F suite pins the same field set in memory; this row really holds all three
    // references, so this is the case that could leak.
    let contract = once.to_contract();
    let mut fields: Vec<String> = serde_json::to_value(&contract)
        .expect("the projection serializes")
        .as_object()
        .expect("a projection object")
        .keys()
        .cloned()
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields.join(" "),
        "base_url connection_key created_at disabled display_name group_key group_path \
         has_read_credential has_webhook_secret mode provider updated_at visibility",
        "the enabled projection is the same presence-only field set as the disabled one"
    );
    let serialized = serde_json::to_string(&contract).expect("the projection serializes");
    for (slot, key) in keys.iter().enumerate() {
        assert!(
            !serialized.contains(key.as_str()),
            "the projection leaks the reference stored in slot {slot}"
        );
    }

    cleanup(&db, &[(group_id, keys)]).await;
}

#[tokio::test]
async fn an_unowned_scope_reaches_neither_the_read_nor_the_update() {
    let Some(db) = scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the enable round trip");
        return;
    };
    let (group_id, keys) = fixture(&db, "enable-owner").await;
    assert!(
        db.get_git_provider_connection(UNOWNED_GROUP, CONNECTION_KEY)
            .await
            .expect("read the unowned scope")
            .is_none(),
        "a scope that owns nothing reads no connection"
    );
    assert!(
        !db.enable_git_provider_connection(UNOWNED_GROUP, CONNECTION_KEY)
            .await
            .expect("attempt an enable from an unowned scope"),
        "an unowned scope matches no row"
    );
    let owned = stored(&db, group_id).await;
    assert!(
        owned.disabled_at.is_none(),
        "an attempt from another scope must not change the owner's connection"
    );

    cleanup(&db, &[(group_id, keys)]).await;
}
