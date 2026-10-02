//! Database-gated round trips for the webhook registration create (issue #681
//! phase 5H).
//!
//! These cover what only storage can show: a supplied synthetic secret is sealed
//! and referenced, an omitted one stores an inactive registration, a refused
//! duplicate writes no secret at all, a hook identity another repository already
//! claims is a conflict that repoints nothing, and two concurrent creates for one
//! repository produce exactly one winner whose sealed value is its own.
//!
//! Every assertion reports a status, a boolean, a count, or a comparison result —
//! no signing secret, ciphertext, store key, derived key, provider id, or
//! repository content is ever printed, and the store is opened only through the
//! phase's own per-run master key. Skipped unless `CONTEXT69_TEST_DATABASE_URL`
//! names a migrated disposable database.

use uuid::Uuid;

use crate::{
    contracts::sources::{GitProviderKind, GitWebhookOwnership, GitWebhookRegistrationRequest},
    services::{
        git_secrets::{GitSecretReader, GitSecretSlot, GitSecretTarget},
        secret_store::SecretPurpose,
    },
};

use super::{
    CreateWebhookFailure, create_group_webhook_registration,
    fixture::{
        UNOWNED_GROUP, cleanup, cleanup_orphans, other_source, read, seeded, store_row_count,
        synthetic_hook_id, synthetic_secret,
    },
};

/// A request for this repository, with or without a synthetic secret.
fn request(signing_secret: Option<String>) -> GitWebhookRegistrationRequest {
    GitWebhookRegistrationRequest {
        provider: GitProviderKind::GitHub,
        external_hook_id: synthetic_hook_id(),
        ownership: GitWebhookOwnership::Integration,
        signing_secret,
    }
}

/// Creates through the route's own transactional path.
async fn create(
    db: &crate::db::Database,
    secrets: &crate::services::secret_store::SecretStore,
    group_id: i64,
    repository_key: Uuid,
    request: &GitWebhookRegistrationRequest,
) -> Result<crate::db::StoredGitWebhookRegistration, CreateWebhookFailure> {
    create_group_webhook_registration(db, secrets, group_id, repository_key, request).await
}

#[tokio::test]
async fn a_supplied_secret_is_sealed_once_and_the_response_reports_only_presence() {
    let Some(seeded) = seeded().await else { return };
    let secret = synthetic_secret();
    let before = store_row_count(&seeded.db).await;
    let stored = create(
        &seeded.db,
        &seeded.secrets,
        seeded.group_id,
        seeded.repository_key,
        &request(Some(secret.clone())),
    )
    .await
    .expect("a first create succeeds");
    assert_eq!(
        store_row_count(&seeded.db).await - before,
        1,
        "exactly one store row is written, for the one supplied secret"
    );
    assert!(stored.active && stored.signing_secret_key.is_some());
    let contract = stored.to_contract();
    assert!(
        contract.has_signing_secret,
        "the response reports presence, never the value"
    );
    // The reference resolves to the repository record's own store row, under the
    // webhook purpose, holding exactly the submitted bytes.
    let reference = stored
        .signing_secret_key
        .clone()
        .expect("a created registration references its sealed secret");
    let target = GitSecretTarget::Repository {
        group_id: seeded.group_id,
        repository_key: seeded.repository_key,
    };
    let opened = GitSecretReader::new(seeded.secrets.clone())
        .read(&target, GitSecretSlot::Webhook)
        .await
        .expect("open the sealed value")
        .expect("it is stored under the same derived name");
    assert_eq!(
        opened.expose(),
        secret.as_bytes(),
        "the sealed value is the submitted one"
    );
    assert!(
        seeded
            .secrets
            .get(SecretPurpose::GitWebhookSigningSecret, &reference)
            .await
            .expect("open the referenced row")
            .is_some(),
        "the row the registration references resolves to a stored value"
    );
    assert_eq!(
        read(&seeded, seeded.group_id, seeded.repository_key)
            .await
            .map(|row| row.external_hook_id),
        Some(contract.external_hook_id),
        "the group-scoped read returns what the create stored"
    );
    cleanup(&seeded, &[reference]).await;
}

#[tokio::test]
async fn an_omitted_secret_stores_an_inactive_registration_and_no_store_row() {
    let Some(seeded) = seeded().await else { return };
    let before = store_row_count(&seeded.db).await;
    let stored = create(
        &seeded.db,
        &seeded.secrets,
        seeded.group_id,
        seeded.repository_key,
        &request(None),
    )
    .await
    .expect("a create without a secret succeeds");
    assert!(
        !stored.active && stored.signing_secret_key.is_none(),
        "an unverifiable hook is stored inactive and reference-less"
    );
    assert!(!stored.to_contract().has_signing_secret);
    assert_eq!(
        store_row_count(&seeded.db).await,
        before,
        "no secret was supplied, so no store row is written"
    );
    cleanup(&seeded, &[]).await;
}

#[tokio::test]
async fn a_refused_duplicate_is_rejected_before_anything_is_sealed() {
    let Some(seeded) = seeded().await else { return };
    let first = create(
        &seeded.db,
        &seeded.secrets,
        seeded.group_id,
        seeded.repository_key,
        &request(Some(synthetic_secret())),
    )
    .await
    .expect("a first create succeeds");
    let reference = first
        .signing_secret_key
        .clone()
        .expect("the first create references its sealed secret");
    let rows_before = store_row_count(&seeded.db).await;

    // Same repository, a different hook id and a different synthetic secret: the
    // pre-check must refuse it before the seal, so nothing is written.
    let duplicate = create(
        &seeded.db,
        &seeded.secrets,
        seeded.group_id,
        seeded.repository_key,
        &request(Some(synthetic_secret())),
    )
    .await;
    assert!(
        matches!(duplicate, Err(CreateWebhookFailure::AlreadyExists)),
        "a second registration on one repository is the bounded conflict"
    );
    assert_eq!(
        store_row_count(&seeded.db).await,
        rows_before,
        "the refused duplicate sealed no value"
    );
    let survivor = read(&seeded, seeded.group_id, seeded.repository_key)
        .await
        .expect("the first registration survives");
    assert_eq!(
        (
            survivor.external_hook_id,
            survivor.signing_secret_key,
            survivor.updated_at
        ),
        (
            first.external_hook_id,
            first.signing_secret_key,
            first.updated_at
        ),
        "the refused duplicate neither overwrote nor repointed the registration"
    );
    cleanup(&seeded, &[reference]).await;
}

#[tokio::test]
async fn a_hook_identity_another_repository_claims_is_a_conflict_that_repoints_nothing() {
    let Some(seeded) = seeded().await else { return };
    let claimed = create(
        &seeded.db,
        &seeded.secrets,
        seeded.group_id,
        seeded.repository_key,
        &request(Some(synthetic_secret())),
    )
    .await
    .expect("a first create succeeds");
    let reference = claimed
        .signing_secret_key
        .clone()
        .expect("the first create references its sealed secret");

    // A second repository of the same group presenting the same provider/hook
    // identity: the unique index refuses it, and the existing registration keeps
    // its own hook id and reference.
    let other_repository = other_source(&seeded.db, seeded.group_id).await;
    let mut collides = request(Some(synthetic_secret()));
    collides.external_hook_id = claimed.external_hook_id.clone();
    assert!(
        matches!(
            create(
                &seeded.db,
                &seeded.secrets,
                seeded.group_id,
                other_repository,
                &collides
            )
            .await,
            Err(CreateWebhookFailure::AlreadyExists)
        ),
        "a hook identity another repository claims is the same bounded conflict"
    );
    assert!(
        read(&seeded, seeded.group_id, other_repository)
            .await
            .is_none(),
        "the losing repository stored no registration"
    );
    let survivor = read(&seeded, seeded.group_id, seeded.repository_key)
        .await
        .expect("the winning registration survives");
    assert_eq!(
        (survivor.external_hook_id, survivor.signing_secret_key),
        (claimed.external_hook_id, claimed.signing_secret_key),
        "the existing registration was neither repointed nor overwritten"
    );
    cleanup(&seeded, &[reference]).await;
}

#[tokio::test]
async fn the_create_only_insert_is_confined_to_a_repository_this_group_owns() {
    let Some(seeded) = seeded().await else { return };
    let owned = create(
        &seeded.db,
        &seeded.secrets,
        seeded.group_id,
        seeded.repository_key,
        &request(Some(synthetic_secret())),
    )
    .await
    .expect("a first create succeeds");
    let reference = owned
        .signing_secret_key
        .clone()
        .expect("the first create references its sealed secret");

    // The create path's own statement is confined through the repository source,
    // so a group that owns nothing selects no row. The route refuses a foreign
    // repository before this point (its wiring order is pinned in the pure
    // suite); driving the path directly proves the statement beneath it cannot
    // write either. Each attempt leaves the store's own reclaimable orphan —
    // bounded, unreferenced, and removed below — and no registration row.
    for repository_key in [seeded.repository_key, UNOWNED_REPOSITORY] {
        let before = store_row_count(&seeded.db).await;
        let outcome = create(
            &seeded.db,
            &seeded.secrets,
            UNOWNED_GROUP,
            repository_key,
            &request(Some(synthetic_secret())),
        )
        .await;
        assert!(
            matches!(outcome, Err(CreateWebhookFailure::Storage(_))),
            "a repository this group does not own inserts no row"
        );
        assert_eq!(
            store_row_count(&seeded.db).await - before,
            1,
            "the only residue is one unreferenced store row the store reclaims"
        );
        assert!(
            read(&seeded, UNOWNED_GROUP, repository_key).await.is_none(),
            "no registration exists for a group that owns no such repository"
        );
    }
    let survivor = read(&seeded, seeded.group_id, seeded.repository_key)
        .await
        .expect("the owning registration survives");
    assert_eq!(
        (
            survivor.external_hook_id,
            survivor.signing_secret_key,
            survivor.updated_at
        ),
        (
            owned.external_hook_id,
            owned.signing_secret_key,
            owned.updated_at
        ),
        "a refused attempt neither moved nor overwrote the owner's registration"
    );
    cleanup(&seeded, &[reference]).await;
    cleanup_orphans(&seeded, UNOWNED_GROUP).await;
}

/// A repository key that names nothing at all, so the insert's own select finds
/// no row even in the owning group.
const UNOWNED_REPOSITORY: Uuid = Uuid::nil();

#[tokio::test]
async fn two_concurrent_creates_produce_one_winner_whose_sealed_value_is_its_own() {
    let Some(seeded) = seeded().await else { return };
    // Two different synthetic secrets against the same repository: the lock must
    // serialize them, so the loser's value can never become the winner's row.
    let secrets = [synthetic_secret(), synthetic_secret()];
    let requests = [
        request(Some(secrets[0].clone())),
        request(Some(secrets[1].clone())),
    ];
    let db = seeded.db.clone();
    let store = seeded.secrets.clone();
    let group_id = seeded.group_id;
    let repository_key = seeded.repository_key;
    let (first, second) = tokio::join!(
        create(&db, &store, group_id, repository_key, &requests[0]),
        create(&db, &store, group_id, repository_key, &requests[1]),
    );
    let outcomes = [first, second];
    let successes = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    assert_eq!(successes, 1, "exactly one create wins the race");
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Err(CreateWebhookFailure::AlreadyExists)))
            .count(),
        1,
        "the other create answers the same bounded conflict"
    );
    let winner = outcomes
        .iter()
        .find_map(|outcome| outcome.as_ref().ok())
        .expect("one stored registration");
    let winner_secret = secrets
        .iter()
        .find(|secret| **secret == request_secret(&requests, winner))
        .expect("the winner submitted one of the two synthetic secrets");
    let target = GitSecretTarget::Repository {
        group_id,
        repository_key,
    };
    let stored_value = GitSecretReader::new(seeded.secrets.clone())
        .read(&target, GitSecretSlot::Webhook)
        .await
        .expect("open the winner's sealed value")
        .expect("the winner's value is stored");
    assert_eq!(
        stored_value.expose(),
        winner_secret.as_bytes(),
        "the stored sealed value is the winner's own, never the loser's"
    );
    let reference = winner
        .signing_secret_key
        .clone()
        .expect("the winner references its sealed secret");
    cleanup(&seeded, &[reference]).await;
}

/// The hook id the winning registration stored, used only to tell the two racing
/// requests apart.
fn request_secret(
    requests: &[GitWebhookRegistrationRequest; 2],
    winner: &crate::db::StoredGitWebhookRegistration,
) -> String {
    requests
        .iter()
        .find(|request| request.external_hook_id == winner.external_hook_id)
        .and_then(|request| request.signing_secret.clone())
        .expect("the winner is one of the two requests")
}
