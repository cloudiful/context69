//! The persistent rebuild barrier and the captured vector identity of the
//! registry (issue #1138 P26).
//!
//! A full vector rebuild rewrites the whole collection, so no registry-admitted
//! publication may overlap it. The barrier is durable and lives in the epoch
//! row every registry writer already locks, so the rule holds across nodes and
//! across a restart: a node that never observed the rebuild reads the same
//! state and fails closed.
//!
//! The captured identity closes the second half of the contract: a run is
//! admitted with the vector space of the node that staged its points, and the
//! switch compares it with the persisted configuration, so a node whose
//! snapshot predates a settings change elsewhere cannot make its points
//! visible.
//!
//! These tests run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and supplies it as
//! `CONTEXT69_TEST_DATABASE_URL`; they are skipped (reported `ignored`) by a
//! plain `cargo test --workspace` and are reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.

#[path = "vector_registry_barrier/support.rs"]
mod support;

use std::panic::AssertUnwindSafe;

use context69::db::{
    AdmissionOutcome, RebuildBarrierState, RebuildRefusalReason, RebuildStartOutcome,
    RegistryQuotaOverride,
};
use context69_test_support::library_policy::{
    LibraryPolicyBaseline, finish_fixture, panic_message,
};
use futures::FutureExt;
use uuid::Uuid;

use support::{
    barrier_state, clear_registry_entries, clear_runtime_settings, connect, fingerprint, identity,
    registry_entry, reset_barrier, seed_runtime_settings, seed_task_item, set_item_lease,
};

const FINGERPRINT: &str = "barrier-fingerprint";
const COLLECTION: &str = "barrier-collection";

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unresolved_staging_entry_blocks_the_rebuild_start() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let mut tx = db.pool().begin().await.expect("begin admission");
    let entry = registry_entry(Uuid::new_v4());
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &entry,
            Some(&identity(FINGERPRINT, COLLECTION)),
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("admit the staging entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    tx.commit().await.expect("commit the admission");

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    let outcome = db
        .begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::StagingEntries),
        "an admitted run that never resolved blocks the rebuild"
    );
    tx.rollback().await.expect("roll back the refused start");
    assert_eq!(
        barrier_state(&db).await,
        RebuildBarrierState::Idle,
        "a refused start never takes the barrier"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_running_barrier_refuses_a_new_admission_and_writes_nothing() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    let outcome = db
        .begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the rebuild start");

    let epoch_before = db.read_search_registry_epoch().await.expect("epoch before");
    let mut tx = db.pool().begin().await.expect("begin admission");
    let entry = registry_entry(Uuid::new_v4());
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &entry,
            Some(&identity(FINGERPRINT, COLLECTION)),
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("admit during the rebuild");
    assert_eq!(
        admitted,
        AdmissionOutcome::RebuildBusy,
        "a rebuild refuses a new admission"
    );
    tx.rollback()
        .await
        .expect("roll back the refused admission");
    assert_eq!(
        db.read_search_registry_epoch().await.expect("epoch after"),
        epoch_before,
        "the refused admission writes nothing"
    );
    assert!(
        !db.read_search_registry_state()
            .await
            .expect("read the registry")
            .entries
            .iter()
            .any(|stored| stored.run_id == entry.run_id),
        "the refused admission leaves no entry"
    );

    // The mutation gate the delete and the metadata publication use refuses
    // the same way.
    let mut tx = db.pool().begin().await.expect("begin mutation");
    assert!(
        db.require_rebuild_idle_in(&mut tx).await.is_err(),
        "a rebuild refuses a registry-visible mutation"
    );
    tx.rollback().await.expect("roll back the refused mutation");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failed_barrier_refuses_everything_until_the_same_identity_rebuilds() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    let mut tx = db.pool().begin().await.expect("begin failure");
    assert!(
        db.fail_rebuild_in(&mut tx, owner, owner_token, "the rebuild failed")
            .await
            .expect("record the failure"),
        "the owner records its own failure"
    );
    tx.commit().await.expect("commit the failure");
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Failed);

    // A failed barrier refuses a new admission.
    let mut tx = db.pool().begin().await.expect("begin admission");
    let entry = registry_entry(Uuid::new_v4());
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &entry,
            Some(&identity(FINGERPRINT, COLLECTION)),
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("admit during the failed rebuild");
    assert_eq!(
        admitted,
        AdmissionOutcome::RebuildBusy,
        "a failed rebuild keeps the registry closed"
    );
    tx.rollback()
        .await
        .expect("roll back the refused admission");

    // A different identity is never taken over automatically.
    let mut tx = db.pool().begin().await.expect("begin other identity");
    let outcome = db
        .begin_rebuild_in(
            &mut tx,
            owner,
            Uuid::new_v4(),
            "other-fingerprint",
            "other-collection",
        )
        .await
        .expect("start a rebuild of another identity");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::IdentityMismatch),
        "another vector identity never clears a failed barrier"
    );
    tx.rollback().await.expect("roll back the refused start");
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Failed);

    // The same identity may retry and complete.
    let (retry_owner, _retry_owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin retry");
    let outcome = db
        .begin_rebuild_in(
            &mut tx,
            retry_owner,
            _retry_owner_token,
            FINGERPRINT,
            COLLECTION,
        )
        .await
        .expect("retry the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the retry");
    let mut tx = db.pool().begin().await.expect("begin completion");
    assert!(
        db.complete_rebuild_in(&mut tx, retry_owner, Uuid::new_v4())
            .await
            .expect("clear the barrier")
            .is_none(),
        "a token that does not own the barrier never clears it"
    );
    tx.rollback()
        .await
        .expect("roll back the refused completion");
    assert_eq!(
        barrier_state(&db).await,
        RebuildBarrierState::Running,
        "a stale owner never clears the barrier"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn only_the_owner_may_clear_the_barrier_and_the_epochs_move_with_it() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // A different owner never clears the barrier.
    let mut tx = db.pool().begin().await.expect("begin foreign completion");
    assert!(
        db.complete_rebuild_in(&mut tx, Uuid::new_v4(), Uuid::new_v4())
            .await
            .expect("clear the barrier")
            .is_none(),
        "a foreign owner never clears the barrier"
    );
    tx.rollback()
        .await
        .expect("roll back the refused completion");
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Running);

    // The owner clears it and the epoch moves with it.
    let epoch_before = db.read_search_registry_epoch().await.expect("epoch before");
    let mut tx = db.pool().begin().await.expect("begin completion");
    let cleared = db
        .complete_rebuild_in(&mut tx, owner, owner_token)
        .await
        .expect("clear the barrier");
    assert_eq!(cleared, Some(epoch_before + 1));
    tx.rollback().await.expect("roll back the completion");
    assert_eq!(
        db.read_search_registry_epoch().await.expect("epoch after"),
        epoch_before,
        "a rolled back completion never moves the epoch"
    );
    assert_eq!(
        barrier_state(&db).await,
        RebuildBarrierState::Running,
        "a rolled back completion never clears the barrier"
    );

    let mut tx = db.pool().begin().await.expect("begin completion");
    db.complete_rebuild_in(&mut tx, owner, owner_token)
        .await
        .expect("clear the barrier");
    tx.commit().await.expect("commit the completion");
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Idle);
    assert_eq!(
        db.read_search_registry_epoch().await.expect("epoch after"),
        epoch_before + 1,
        "the completion moves the epoch with the barrier"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_dead_owner_is_taken_over_by_the_same_identity_only() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    // An owner that is no longer running can never pass its grant check
    // again, so its barrier may be taken over.
    let (dead_owner, _dead_owner_token) = seed_task_item(&db, "failed").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        dead_owner,
        _dead_owner_token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    let (next_owner, _next_owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin takeover");
    let outcome = db
        .begin_rebuild_in(
            &mut tx,
            next_owner,
            _next_owner_token,
            FINGERPRINT,
            COLLECTION,
        )
        .await
        .expect("take the barrier over");
    assert!(
        matches!(outcome, RebuildStartOutcome::Started(_)),
        "the same identity takes a dead owner's barrier over"
    );
    tx.commit().await.expect("commit the takeover");
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert_eq!(
        barrier.owner_item_id,
        Some(next_owner),
        "the new owner is recorded"
    );

    // A live owner keeps the barrier.
    let mut tx = db.pool().begin().await.expect("begin refusal");
    let outcome = db
        .begin_rebuild_in(
            &mut tx,
            seed_task_item(&db, "running").await.0,
            Uuid::new_v4(),
            FINGERPRINT,
            COLLECTION,
        )
        .await
        .expect("start a rebuild");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::Busy),
        "a live owner keeps the barrier"
    );
    tx.rollback().await.expect("roll back the refused start");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_identity_and_the_barrier_commit_and_roll_back_together() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // A rolled back completion writes neither the identity nor the barrier.
    let mut tx = db.pool().begin().await.expect("begin completion");
    db.save_vector_index_state_in(
        &mut tx,
        &context69::db::VectorIndexState {
            collection_name: COLLECTION,
            fingerprint: FINGERPRINT,
            embedding_base_url: "https://embeddings.example/v1",
            embedding_model: "model-a",
            dimensions: 8,
            rebuilt_chunks: 12,
        },
    )
    .await
    .expect("write the identity");
    db.complete_rebuild_in(&mut tx, owner, owner_token)
        .await
        .expect("clear the barrier");
    tx.rollback().await.expect("roll back the completion");
    assert_eq!(
        db.get_vector_index_fingerprint(COLLECTION)
            .await
            .expect("read the identity"),
        None,
        "a rolled back completion writes no identity"
    );
    assert_eq!(
        barrier_state(&db).await,
        RebuildBarrierState::Running,
        "a rolled back completion leaves the barrier"
    );

    // The committed completion writes both.
    let mut tx = db.pool().begin().await.expect("begin completion");
    db.save_vector_index_state_in(
        &mut tx,
        &context69::db::VectorIndexState {
            collection_name: COLLECTION,
            fingerprint: FINGERPRINT,
            embedding_base_url: "https://embeddings.example/v1",
            embedding_model: "model-a",
            dimensions: 8,
            rebuilt_chunks: 12,
        },
    )
    .await
    .expect("write the identity");
    db.complete_rebuild_in(&mut tx, owner, owner_token)
        .await
        .expect("clear the barrier");
    tx.commit().await.expect("commit the completion");
    assert_eq!(
        db.get_vector_index_fingerprint(COLLECTION)
            .await
            .expect("read the identity"),
        Some(FINGERPRINT.to_string()),
        "the committed completion writes the identity"
    );
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Idle);
    sqlx::query("DELETE FROM context69.vector_index_state WHERE collection_name = $1")
        .bind(COLLECTION)
        .execute(db.pool())
        .await
        .expect("clear the recorded identity");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_stale_admitted_identity_cannot_switch() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;

    let mut tx = db.pool().begin().await.expect("begin admission");
    let entry = registry_entry(Uuid::new_v4());
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &entry,
            Some(&identity(
                &fingerprint(
                    "http://qdrant.example:6333",
                    COLLECTION,
                    "https://embeddings.example/v1",
                    "model-a",
                    8,
                ),
                COLLECTION,
            )),
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("admit the entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    tx.commit().await.expect("commit the admission");

    // The configuration still describes the admitted identity, so the switch
    // is allowed.
    let mut tx = db.pool().begin().await.expect("begin switch");
    assert!(
        db.require_matching_vector_identity_in(&mut tx, entry.run_id)
            .await
            .is_ok(),
        "an unchanged configuration keeps the switch allowed"
    );
    tx.rollback().await.expect("roll back the switch");

    // A configuration change on another node moves the identity, and the
    // stale run may not publish its points.
    seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-b",
        8,
    )
    .await;
    let mut tx = db.pool().begin().await.expect("begin switch");
    let refused = db
        .require_matching_vector_identity_in(&mut tx, entry.run_id)
        .await;
    assert!(
        refused.is_err(),
        "a changed configuration refuses the stale run"
    );
    tx.rollback().await.expect("roll back the refused switch");

    // The collection is part of the identity as well.
    seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        "another-collection",
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;
    let mut tx = db.pool().begin().await.expect("begin switch");
    assert!(
        db.require_matching_vector_identity_in(&mut tx, entry.run_id)
            .await
            .is_err(),
        "another collection refuses the stale run"
    );
    tx.rollback().await.expect("roll back the refused switch");

    clear_runtime_settings(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_readiness_of_the_deployment_observes_the_barrier() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;

    let settings = context69::services::settings::SettingsService::new(db.clone());
    let response = settings
        .get_runtime_settings()
        .await
        .expect("read the runtime settings");
    assert!(
        !response.requires_rebuild,
        "an idle barrier needs no rebuild"
    );

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // A fresh service observes the barrier: the rule does not depend on an
    // in-process flag, so a node that never saw the rebuild fails closed too.
    let settings = context69::services::settings::SettingsService::new(db.clone());
    let response = settings
        .get_runtime_settings()
        .await
        .expect("read the runtime settings");
    assert!(
        response.requires_rebuild,
        "a running barrier is a rebuild this deployment owes"
    );

    let mut tx = db.pool().begin().await.expect("begin completion");
    db.complete_rebuild_in(&mut tx, owner, Uuid::new_v4())
        .await
        .expect("clear the barrier");
    tx.rollback()
        .await
        .expect("roll back the refused completion");

    clear_runtime_settings(&db).await;
}

/// A superseded owner token releases the barrier, an elapsed lease does not.
///
/// The recorded owner of a running barrier is the fenced task item that owns
/// it, and the recorded token is that owner's grant. A reclaimed item carries a
/// fresh token, so the recorded owner provably cannot write any more; an
/// elapsed deadline proves nothing of the sort and keeps the barrier.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_superseded_owner_token_releases_the_barrier_but_an_elapsed_lease_does_not() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    let outcome = db
        .begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the rebuild start");

    // The recorded owner still holds its grant, so the barrier stays.
    let mut tx = db.pool().begin().await.expect("begin attempt");
    let outcome = db
        .begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("retry the rebuild");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::Busy),
        "a live owner keeps the barrier"
    );
    tx.rollback().await.expect("roll back the refused attempt");

    // An elapsed lease deadline is not proof that the owner is gone.
    set_item_lease(&db, owner, owner_token, true).await;
    let mut tx = db.pool().begin().await.expect("begin attempt");
    let outcome = db
        .begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("retry the rebuild");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::Busy),
        "an elapsed deadline never releases the barrier"
    );
    tx.rollback().await.expect("roll back the refused attempt");

    // A reclaimed item carries a fresh token: the recorded owner provably
    // cannot write, so the same identity may take the barrier over.
    set_item_lease(&db, owner, Uuid::new_v4(), false).await;
    let (next_owner, _next_owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin takeover");
    let outcome = db
        .begin_rebuild_in(
            &mut tx,
            next_owner,
            _next_owner_token,
            FINGERPRINT,
            COLLECTION,
        )
        .await
        .expect("take the barrier over");
    assert!(
        matches!(outcome, RebuildStartOutcome::Started(_)),
        "a superseded owner releases the barrier for the same identity"
    );
    tx.commit().await.expect("commit the takeover");

    // A different identity is still refused.
    let mut tx = db.pool().begin().await.expect("begin attempt");
    let outcome = db
        .begin_rebuild_in(
            &mut tx,
            next_owner,
            _next_owner_token,
            "other-fingerprint",
            "other-collection",
        )
        .await
        .expect("start another identity");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::IdentityMismatch),
        "another vector identity never takes the barrier over"
    );
    tx.rollback().await.expect("roll back the refused attempt");
}

/// The settings save refuses an identity change while a rebuild owns the
/// barrier or an unresolved run holds staged points, and it decides inside its
/// own transaction.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_settings_save_refuses_an_identity_change_a_rebuild_or_a_staged_run_owns() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    let queue = db
        .execution_queue()
        .await
        .expect("load the execution queue");
    let baseline = LibraryPolicyBaseline::capture(queue)
        .await
        .expect("capture the execution policy baseline");

    let outcome = AssertUnwindSafe(async {
        seed_runtime_settings(
            &db,
            "http://qdrant.example:6333",
            COLLECTION,
            "https://embeddings.example/v1",
            "model-a",
            8,
        )
        .await;

        let stored = db
            .get_runtime_settings()
            .await
            .expect("read the runtime settings")
            .expect("the runtime settings exist");

        // A credential or timeout change does not move the vector space, so it is
        // never blocked by a rebuild or by a staged run.
        let mut credential_only = stored.clone();
        credential_only.embedding.timeout_secs = 42;
        db.save_runtime_settings(&credential_only)
            .await
            .expect("a credential-only save is never blocked");

        // An identity change is refused while an unresolved run holds staged
        // points.
        let entry = registry_entry(Uuid::new_v4());
        let mut tx = db.pool().begin().await.expect("begin admission");
        let admitted = db
            .admit_search_registry_entry_checked_in(
                &entry,
                Some(&identity(
                    &fingerprint(
                        "http://qdrant.example:6333",
                        COLLECTION,
                        "https://embeddings.example/v1",
                        "model-a",
                        8,
                    ),
                    COLLECTION,
                )),
                0,
                RegistryQuotaOverride::exact([1000, 1000, 1000]),
                &mut tx,
            )
            .await
            .expect("admit the staging entry");
        assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
        tx.commit().await.expect("commit the admission");

        let mut changed = stored.clone();
        changed.embedding.model = "model-b".to_string();
        assert!(
            db.save_runtime_settings(&changed).await.is_err(),
            "a staged run refuses an identity change"
        );

        // The stored configuration is untouched by the refused save.
        let after = db
            .get_runtime_settings()
            .await
            .expect("read the runtime settings")
            .expect("the runtime settings exist");
        assert_eq!(
            after.embedding.model, stored.embedding.model,
            "the refused save changed nothing"
        );

        // Once the run resolved, the same change is allowed.
        let mut tx = db.pool().begin().await.expect("begin transition");
        db.transition_search_registry_entry_in(entry.run_id, "staging", "retired", None, &mut tx)
            .await
            .expect("retire the staging entry");
        tx.commit().await.expect("commit the transition");
        let (stale_owner, stale_token) = seed_task_item(&db, "running").await;
        let mut rebuild_tx = db.pool().begin().await.expect("begin the stale rebuild");
        let captured = db
            .read_persisted_vector_identity_in(&mut rebuild_tx)
            .await
            .expect("read the identity the rebuild captured")
            .expect("the identity exists");
        db.save_runtime_settings(&changed)
            .await
            .expect("the identity change is allowed once no run holds staged points");
        // The authoritative begin reads the persisted identity after it takes the
        // epoch lock, so the commit above must be visible as a changed identity.
        let stale_start = db
            .begin_rebuild_checked_in(
                &mut rebuild_tx,
                stale_owner,
                stale_token,
                &captured.embedding_fingerprint,
                &captured.collection_name,
            )
            .await
            .expect("attempt admission after the intervening settings commit");
        rebuild_tx
            .rollback()
            .await
            .expect("leave the barrier unchanged");
        assert!(
            matches!(stale_start, RebuildStartOutcome::Refused(_)),
            "a settings commit between the service identity read and the epoch lock must refuse the stale rebuild: {stale_start:?}"
        );

        // A destination change is refused while a rebuild owns the barrier.
        let (owner, owner_token) = seed_task_item(&db, "running").await;
        let mut tx = db.pool().begin().await.expect("begin rebuild");
        db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
            .await
            .expect("start the rebuild");
        tx.commit().await.expect("commit the rebuild start");
        let mut destination = changed.clone();
        destination.qdrant.collection_name = "another-collection".to_string();
        assert!(
            db.save_runtime_settings(&destination).await.is_err(),
            "a running rebuild refuses a destination change"
        );

        // A failed rebuild keeps the barrier closed as well.
        let mut tx = db.pool().begin().await.expect("begin failure");
        db.fail_rebuild_in(&mut tx, owner, Uuid::new_v4(), "the rebuild failed")
            .await
            .expect("record the failure");
        tx.commit().await.expect("commit the failure");
        assert!(
            db.save_runtime_settings(&destination).await.is_err(),
            "a failed rebuild refuses a destination change"
        );
    })
    .catch_unwind()
    .await;

    let cleanup = AssertUnwindSafe(async {
        clear_runtime_settings(&db).await;
    })
    .catch_unwind()
    .await;
    let cleanup_error = cleanup.err().map(panic_message);
    let restore = baseline.restore(db.pool(), queue).await;
    finish_fixture(outcome, cleanup_error, restore);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_completion_waits_for_the_epoch_before_it_writes_the_identity() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // Another connection holds the epoch.
    let mut gate = db.pool().begin().await.expect("begin the gate");
    db.lock_search_registry_epoch_in(&mut gate)
        .await
        .expect("lock the epoch");

    let completion = {
        let db = db.clone();
        tokio::spawn(async move {
            let state = context69::db::VectorIndexState {
                collection_name: COLLECTION,
                fingerprint: FINGERPRINT,
                embedding_base_url: "https://embeddings.example/v1",
                embedding_model: "model-a",
                dimensions: 8,
                rebuilt_chunks: 3,
            };
            let mut tx = db.pool().begin().await.expect("begin the completion");
            let cleared = db
                .complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
                .await;
            match cleared {
                Ok(cleared) => {
                    tx.commit().await.expect("commit the completion");
                    Ok(cleared)
                }
                Err(error) => {
                    let _ = tx.rollback().await;
                    Err(error)
                }
            }
        })
    };

    // While the epoch is held, the completion may not write the identity.
    for _ in 0..50 {
        if db
            .get_vector_index_fingerprint(COLLECTION)
            .await
            .expect("read the identity")
            .is_some()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        db.get_vector_index_fingerprint(COLLECTION)
            .await
            .expect("read the identity")
            .is_none(),
        "the completion waits for the epoch before it writes the identity"
    );

    gate.rollback().await.expect("release the epoch");
    let cleared = completion
        .await
        .expect("join the completion")
        .expect("complete the rebuild");
    assert!(cleared.is_some(), "the owner clears the barrier");
    assert_eq!(
        db.get_vector_index_fingerprint(COLLECTION)
            .await
            .expect("read the identity"),
        Some(FINGERPRINT.to_string()),
        "the completion writes the identity it re-embedded with"
    );
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Idle);

    sqlx::query("DELETE FROM context69.vector_index_state WHERE collection_name = $1")
        .bind(COLLECTION)
        .execute(db.pool())
        .await
        .expect("clear the recorded identity");
}
