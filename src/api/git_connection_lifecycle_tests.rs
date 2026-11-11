//! Pure disable-route validation and wiring tests.

use chrono::{DateTime, Utc};

use crate::api::group_access::require_group_role;
use crate::contracts::sources::{GitConnectionMode, GitProviderKind};
use crate::contracts::{GroupKind, MembershipRole, Visibility};
use crate::db::{GitGroupOwnership, StoredGitProviderConnection};
use crate::domain::GroupRecord;

/// The compiled route module, so the wiring below is asserted against the real
/// handler rather than a copy of it.
const ROUTE_SOURCE: &str = include_str!("git_connection_lifecycle.rs");

/// A synthetic key; nothing here is a credential of any deployment.
const CONNECTION_KEY: &str = "github-app";

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
/// it reaches the update.
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
    assert_eq!(
        body.matches("get_git_provider_connection").count(),
        1,
        "the handler reads the connection exactly once, after the update"
    );
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
