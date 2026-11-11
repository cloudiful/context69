//! Pure enable-route validation and wiring tests.

use chrono::{DateTime, Utc};

use crate::api::group_access::require_group_role;
use crate::contracts::{GroupKind, MembershipRole, Visibility};
use crate::domain::GroupRecord;

/// The compiled route module, so the wiring below is asserted against the real
/// handler rather than against a copy of it.
const ROUTE_SOURCE: &str = include_str!("git_connection_lifecycle.rs");

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
