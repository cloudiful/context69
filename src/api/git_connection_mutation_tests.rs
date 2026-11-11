//! Pure connection-create mapping and wiring tests.

use super::is_unique_violation;

/// The route source, so a wiring assertion reads the same file the compiler
/// builds without standing up a full application/router harness.
const ROUTE_SOURCE: &str = include_str!("git_connection_mutations.rs");

#[test]
fn only_a_postgres_unique_violation_is_mapped_to_a_conflict() {
    assert!(!is_unique_violation(&anyhow::anyhow!(
        "plain storage failure: connection refused"
    )));
    let wrapped = anyhow::anyhow!("plain storage failure").context("outer context");
    assert!(!is_unique_violation(&wrapped));
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
