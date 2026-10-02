//! Pure tests for the Maintainer-gated webhook registration route (issue #681
//! phase 5H).
//!
//! Every decision the route makes before it needs a database lives here: the
//! bounded request validation, the provider match, the Maintainer floor, the
//! shared not-found and conflict shapes, the presence-only projection, and the
//! wiring order that keeps a secret write behind a duplicate check. The request's
//! own wire shape belongs to the contract crate's suite, and the round trips
//! against storage are in `git_webhook_registration_db_tests.rs`.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    contracts::{
        GroupKind, MembershipRole, Visibility,
        sources::{
            GIT_WEBHOOK_HOOK_ID_MAX_CHARS, GitProviderKind, GitWebhookOwnership,
            GitWebhookRegistration, GitWebhookRegistrationRejection, GitWebhookRegistrationRequest,
        },
    },
    db::StoredGitWebhookRegistration,
    domain::GroupRecord,
    services::git_webhook_signature::MAX_GIT_WEBHOOK_HOOK_ID_CHARS,
};

use super::{invalid_request, provider_matches, registration_conflict, repository_not_found};

/// The compiled route module, so every wiring assertion below is made against
/// the real handler rather than a copy of it.
const ROUTE_SOURCE: &str = include_str!("git_webhook_registration.rs");
const LOCK_SQL: &str =
    include_str!("../sql/db/git_repositories/acquire_git_webhook_registration_creation_lock.sql");
const INSERT_SQL: &str =
    include_str!("../sql/db/git_repositories/insert_git_webhook_registration.sql");
/// The presence-only field set the create response and the existing read share.
const PROJECTION_FIELDS: &str = "active created_at external_hook_id has_signing_secret \
     ownership provider repository_key updated_at";
/// A fixed instant, so the stored builders are comparable.
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

/// A registration request with a per-run synthetic secret, or without one.
fn request(signing_secret: Option<String>) -> GitWebhookRegistrationRequest {
    GitWebhookRegistrationRequest {
        provider: GitProviderKind::GitHub,
        external_hook_id: "hook-42".to_string(),
        ownership: GitWebhookOwnership::Integration,
        signing_secret,
    }
}

/// The stored registration the create path returns, carrying a reference.
fn stored() -> StoredGitWebhookRegistration {
    StoredGitWebhookRegistration {
        repository_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        provider: GitProviderKind::GitHub,
        external_hook_id: "hook-42".to_string(),
        ownership: GitWebhookOwnership::Integration,
        active: true,
        signing_secret_key: Some("synthetic-signing-secret-reference".to_string()),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

#[test]
fn the_hook_id_bound_is_the_ingress_bound() {
    // Two independent statements of one limit: the request contract and the
    // ingress path segment, so no hook id can be stored that ingress refuses.
    assert_eq!(
        GIT_WEBHOOK_HOOK_ID_MAX_CHARS, MAX_GIT_WEBHOOK_HOOK_ID_CHARS,
        "the create bound and the ingress bound must stay the same bound"
    );
    let mut bounded = request(None);
    bounded.external_hook_id = "h".repeat(GIT_WEBHOOK_HOOK_ID_MAX_CHARS);
    assert!(
        bounded.validate_for_create().is_ok(),
        "a hook id inside the bound is accepted"
    );
    bounded.external_hook_id.push('h');
    assert_eq!(
        bounded.validate_for_create().err(),
        Some(GitWebhookRegistrationRejection::HookIdTooLong),
        "one character past the ingress bound is refused, not trimmed into a \
         different id"
    );
}

#[test]
fn a_blank_hook_id_or_a_blank_supplied_secret_is_refused_with_a_bounded_reason() {
    for hook_id in ["", " ", "\t"] {
        let mut refused = request(None);
        refused.external_hook_id = hook_id.to_string();
        assert_eq!(
            refused.validate_for_create().err(),
            Some(GitWebhookRegistrationRejection::HookIdBlank),
            "a blank hook id names no hook: {hook_id:?}"
        );
    }
    // A blank *supplied* secret is a contradiction — asking to set a secret while
    // sending nothing — so it is refused rather than storing an inactive row.
    for blank in ["", "   "] {
        let refused = request(Some(blank.to_string()));
        assert_eq!(
            refused.validate_for_create().err(),
            Some(GitWebhookRegistrationRejection::SigningSecretBlank),
            "a blank supplied secret must be refused"
        );
        assert!(
            refused.signing_secret_value().is_none(),
            "a refused blank value never reaches the seal"
        );
    }
    for rejection in [
        GitWebhookRegistrationRejection::HookIdBlank,
        GitWebhookRegistrationRejection::HookIdTooLong,
        GitWebhookRegistrationRejection::SigningSecretBlank,
    ] {
        let reason = rejection.as_str();
        assert!(
            reason.starts_with("git_webhook_") && !reason.contains(' '),
            "each reason is one stable bounded token: {reason}"
        );
        // The refusal reaches the wire as a bounded invalid-argument response.
        assert_eq!(
            invalid_request(reason).status(),
            StatusCode::BAD_REQUEST,
            "a refused request answers 400, never 409 or 404"
        );
    }
}

#[test]
fn an_omitted_secret_stores_an_inactive_registration_and_a_supplied_one_an_active_one() {
    let without = request(None);
    assert_eq!(without.signing_secret_value(), None);
    assert!(!without.is_active());

    // Non-blank bytes are sealed exactly as submitted, whitespace included: a
    // provider signs over the exact configured secret, so a trimmed value would
    // store a secret no delivery verifies against.
    let padded = format!("  {}  ", Uuid::new_v4());
    let with = request(Some(padded.clone()));
    assert_eq!(with.signing_secret_value(), Some(padded.as_str()));
    assert!(with.is_active());
    assert!(
        !request(Some(" \t ".to_string())).is_active(),
        "a whitespace-only value is blank, not a secret"
    );
}

#[test]
fn a_hook_for_another_provider_is_a_conflict_and_never_reaches_the_seal() {
    assert!(
        provider_matches(GitProviderKind::GitHub, GitProviderKind::GitHub).is_ok(),
        "the repository's own provider is accepted"
    );
    for other in [
        GitProviderKind::Forgejo,
        GitProviderKind::GitLab,
        GitProviderKind::Generic,
    ] {
        let error = provider_matches(other, GitProviderKind::GitHub)
            .expect_err("a foreign provider is refused");
        assert_eq!(
            error.to_string(),
            "git_webhook_provider_mismatch",
            "the mismatch is one bounded reason"
        );
    }
    let handler = function("pub(crate) async fn create_git_webhook_registration(");
    let checked = handler
        .find("provider_matches")
        .expect("the provider is checked");
    let created = handler
        .find("create_group_webhook_registration(")
        .expect("the create runs through the transactional path");
    assert!(
        checked < created,
        "the provider match precedes the create, so no secret is sealed for it"
    );
}

#[test]
fn the_maintainer_floor_precedes_every_repository_and_secret_step() {
    for role in [MembershipRole::Maintainer, MembershipRole::Owner] {
        crate::api::group_access::require_group_role(
            &group(Some(role)),
            MembershipRole::Maintainer,
        )
        .unwrap_or_else(|error| panic!("{role:?} may register a webhook: {error}"));
    }
    for role in [Some(MembershipRole::Viewer), None] {
        assert!(
            crate::api::group_access::require_group_role(&group(role), MembershipRole::Maintainer)
                .is_err(),
            "a Viewer or non-member is refused before any repository or secret work"
        );
    }
    assert!(
        ROUTE_SOURCE.contains("maintainer_group(&state, session.user.id, &group_path)"),
        "the handler resolves the group through the shared helper"
    );
    // The order is the guarantee: authorization, then the bounded request, then
    // the group-owned repository, then the create.
    let handler = function("pub(crate) async fn create_git_webhook_registration(");
    let bounded = handler
        .find("validate_for_create()")
        .expect("the request is validated");
    let owned = handler
        .find("owned_source(&state")
        .expect("the repository is read group-scoped");
    let created = handler
        .find("create_group_webhook_registration(")
        .expect("the create runs last");
    assert!(
        bounded < owned && owned < created,
        "a refused request never reaches the repository read or the secret store"
    );
}

#[test]
fn the_seal_target_is_the_repository_record_and_never_the_caller_hook_id() {
    // The deterministic store key is derived from the group-qualified repository
    // record, so a submitted hook id can never aim the write at another record.
    let seal = function("async fn seal_signing_secret(");
    assert!(
        seal.contains("GitSecretTarget::Repository"),
        "the sealed value is stored against the repository record"
    );
    assert!(
        !seal.contains("external_hook_id"),
        "the caller-supplied hook id never reaches the secret target"
    );
    assert!(
        !seal.contains(".write(") && seal.contains(".seal("),
        "only the seal-only seam is used: the create never repoints a column"
    );
}
#[tokio::test]
async fn a_foreign_repository_and_a_hook_that_already_exists_share_the_bounded_shapes() {
    // An unknown and a foreign repository share the one not-found shape the other
    // Git reads return, so this route cannot enumerate repository keys.
    let not_found = repository_not_found();
    assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
    let not_found_body = body(not_found).await;
    assert!(
        not_found_body.contains("unknown git repository"),
        "the repository shape is shared with the other reads: {not_found_body}"
    );
    // A registration already on this repository, and a hook identity another
    // repository already claims, both answer with this one conflict.
    let conflict = registration_conflict();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conflict_body = body(conflict).await;
    assert!(
        conflict_body.contains("git_webhook_registration_already_exists"),
        "the conflict names one bounded reason: {conflict_body}"
    );
    for detail in "hook-42 team-platform api.github.com synthetic".split_whitespace() {
        assert!(
            !conflict_body.contains(detail) && !not_found_body.contains(detail),
            "a bounded error never echoes request detail: {detail}"
        );
    }
}
#[test]
fn the_response_is_the_presence_only_registration_projection() {
    // The same projection the existing GET read returns, so a create and a read
    // describe the registration the same way.
    let contract: GitWebhookRegistration = stored().to_contract();
    let encoded = serde_json::to_value(&contract).expect("the projection serializes");
    let mut fields: Vec<&str> = encoded
        .as_object()
        .expect("a projection object")
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields.join(" "),
        PROJECTION_FIELDS,
        "the create response is the exact presence-only field set"
    );
    assert!(
        contract.has_signing_secret,
        "a sealed value is reported as presence, never as a value"
    );
    let serialized = serde_json::to_string(&contract).expect("the projection serializes");
    for forbidden in "signing_secret_key synthetic-signing-secret-reference internal_secrets g42."
        .split_whitespace()
    {
        assert!(
            !serialized.contains(forbidden),
            "the projection must not carry {forbidden}"
        );
    }
    // A registration created without a secret is inactive and reference-less, and
    // projects the same way.
    let mut without = stored();
    without.signing_secret_key = None;
    without.active = false;
    let inactive = without.to_contract();
    assert!(!inactive.has_signing_secret && !inactive.active);
    assert_eq!(
        serde_json::to_value(&inactive)
            .expect("the projection serializes")
            .as_object()
            .expect("a projection object")
            .len(),
        fields.len(),
        "presence, not the presence's value, is the only difference"
    );
}

#[test]
fn the_create_uses_a_transaction_scoped_lock_and_a_create_only_insert() {
    // Transaction-scoped, so PostgreSQL releases it when the create transaction
    // ends and an abandoned create cannot leave a repository locked forever.
    assert!(
        LOCK_SQL.contains("pg_advisory_xact_lock(") && !LOCK_SQL.contains("pg_advisory_lock("),
        "the lock is transaction-scoped, never session-scoped"
    );
    // Create-only: no conflict arm, so a duplicate raises the unique violation the
    // route maps to the bounded conflict instead of overwriting a registration.
    // Comments are stripped so the assertion reads the SQL that actually runs.
    let executable: String = INSERT_SQL
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !executable.contains("ON CONFLICT") && !executable.contains("DO UPDATE"),
        "the insert must have no conflict arm at all"
    );
    assert!(
        executable.contains("s.repository_key = $1") && executable.contains("s.group_id = $2"),
        "the insert writes only onto a repository this group owns"
    );
    let create = function("pub(crate) async fn create_group_webhook_registration(");
    let lock = create
        .find("begin_git_webhook_registration_creation")
        .expect("the create takes the lock");
    let precheck = create
        .find("creation.get(group_id, repository_key)")
        .expect("the duplicate is pre-checked under the lock");
    let seal = create
        .find("seal_signing_secret(")
        .expect("the secret is sealed under the lock");
    let insert = create
        .find("creation.insert(group_id, &new)")
        .expect("the row is inserted under the lock");
    assert!(
        lock < precheck && precheck < seal && seal < insert,
        "lock, pre-check, seal, insert: a duplicate is refused before any secret \
         is sealed"
    );
}

/// One function's body by signature, so an ordering or content assertion is made
/// against that function instead of a file slice a neighbour could satisfy.
fn function(signature: &str) -> &'static str {
    let start = ROUTE_SOURCE.find(signature).expect("the named function");
    let rest = &ROUTE_SOURCE[start..];
    let end = rest[1..]
        .find("\n}\n")
        .map(|offset| offset + 2)
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The UTF-8 body of a bounded response.
async fn body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the bounded body");
    String::from_utf8(bytes.to_vec()).expect("a UTF-8 body")
}
