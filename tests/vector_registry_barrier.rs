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

use context69::db::{
    AdmissionOutcome, RebuildBarrierState, RebuildCursor, RebuildRefusalReason,
    RebuildStartOutcome, RegistryQuotaOverride,
};
use uuid::Uuid;

use support::{
    activate_index, barrier_state, clear_registry_entries, clear_runtime_settings, connect,
    delete_tasks, fingerprint, identity, registry_entry, reset_barrier, seed_runtime_settings,
    seed_task_item, seed_task_item_full, set_item_lease,
};

const FINGERPRINT: &str = "barrier-fingerprint";
const COLLECTION: &str = "barrier-collection";

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unresolved_staging_entry_blocks_the_rebuild_start() {
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
    // The staged entry is this case's own fixture: it is cleared again, so an
    // unresolved run of this target never blocks another suite that has to
    // move the vector space.
    clear_registry_entries(&db).await;
    clear_runtime_settings(&db).await;
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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

    // A configuration change on another node moves the desired identity while
    // the active pointer keeps the old vector space, and the stale run may not
    // publish its points.
    seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-b",
        8,
    )
    .await;
    activate_index(
        &db,
        COLLECTION,
        &fingerprint(
            "http://qdrant.example:6333",
            COLLECTION,
            "https://embeddings.example/v1",
            "model-a",
            8,
        ),
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;
    let mut tx = db.pool().begin().await.expect("begin switch");
    let refused = db
        .require_matching_vector_identity_in(&mut tx, entry.run_id)
        .await;
    assert!(
        refused.is_err(),
        "a changed desired configuration refuses the stale run"
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
    activate_index(
        &db,
        COLLECTION,
        &fingerprint(
            "http://qdrant.example:6333",
            COLLECTION,
            "https://embeddings.example/v1",
            "model-a",
            8,
        ),
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
        "another desired collection refuses the stale run"
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
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
    let (stale_start, _resumed) = db
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
}

/// The completion waits for the epoch before it writes the identity.
///
/// The canonical order is item, epoch, vector identity row, and the settings
/// save takes the same order. A completion that wrote the identity row before
/// it took the epoch could deadlock against a settings save that holds the
/// epoch and waits for that row.
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
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_paused_rebuild_keeps_its_cursor_and_only_an_explicit_resume_reopens_it() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let seeded = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        seeded.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    let mut connection = db.pool().acquire().await.expect("acquire");
    assert!(
        db.advance_rebuild_cursor_in(
            &mut connection,
            seeded.item_id,
            seeded.token,
            FINGERPRINT,
            COLLECTION,
            context69::db::RebuildCheckpoint {
                cursor: RebuildCursor {
                    document_id: 7,
                    chunk_index: 3,
                },
                processed_chunks: 2,
                total_chunks: 9,
            },
        )
        .await
        .expect("advance the cursor"),
        "the owner advances its committed cursor"
    );
    assert!(
        db.request_rebuild_pause_in(&mut connection, seeded.item_id, seeded.token)
            .await
            .expect("request the pause"),
        "the owner requests the pause"
    );
    drop(connection);

    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(barrier.pause_requested);
    assert_eq!(
        barrier.cursor,
        Some(RebuildCursor {
            document_id: 7,
            chunk_index: 3,
        }),
        "a pause keeps the committed cursor"
    );
    assert_eq!(barrier.processed_chunks, 2);
    assert_eq!(barrier.total_chunks, 9);

    // The barrier is durable: a new handle after a restart observes the pause
    // and the same cursor.
    let restarted = connect().await;
    let barrier = restarted
        .read_rebuild_barrier()
        .await
        .expect("read the barrier after a restart");
    assert!(barrier.pause_requested, "the pause survives a restart");
    assert_eq!(
        barrier.cursor,
        Some(RebuildCursor {
            document_id: 7,
            chunk_index: 3,
        }),
        "the committed cursor survives a restart"
    );

    // A paused rebuild is never taken over or restarted by another owner.
    let other = seed_task_item_full(&db, "running", "file_batch").await;
    let mut tx = db.pool().begin().await.expect("begin takeover");
    let outcome = db
        .begin_rebuild_in(&mut tx, other.item_id, other.token, FINGERPRINT, COLLECTION)
        .await
        .expect("try to take the paused barrier over");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::Busy),
        "a paused barrier refuses a new start"
    );
    tx.rollback().await.expect("roll back the refused start");

    // The worker parks the item on the exact paused dependency. The dispatcher
    // must never claim it, and the pause must not spend an attempt.
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', waiting_reason = 'dependency', \
             dependency_key = 'vector_rebuild_paused', lease_token = NULL, \
             lease_until = NULL, next_attempt_at = now() + interval '1 day' \
         WHERE id = $1",
    )
    .bind(seeded.item_id)
    .execute(db.pool())
    .await
    .expect("park the paused item");
    let claimed = db.claim_items(100).await.expect("claim while paused");
    assert!(
        !claimed.iter().any(|item| item.id == seeded.item_id),
        "the dispatcher never claims a parked paused rebuild"
    );

    // Only the explicit resume requeues the same item, recharges its attempt and
    // keeps the committed cursor.
    assert!(
        db.resume_paused_rebuild_item(seeded.task_id)
            .await
            .expect("resume the paused item"),
        "the paused owner is resumed"
    );
    let item = db
        .list_task_items(seeded.task_id, 10, 0)
        .await
        .expect("read the resumed item")
        .into_iter()
        .next()
        .expect("the resumed item exists");
    assert_eq!(item.status, "queued");
    assert_eq!(item.stage.as_deref(), Some("processing"));
    assert_eq!(
        item.attempt_count, 0,
        "the explicit resume recharges the attempt, the paused wait never spent one"
    );
    assert_eq!(item.dependency_key, None);
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(!barrier.pause_requested, "the resume clears the pause flag");
    assert_eq!(
        barrier.cursor,
        Some(RebuildCursor {
            document_id: 7,
            chunk_index: 3,
        }),
        "the resume never resets the committed cursor"
    );

    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_idle_claim_never_attaches_a_target_recorded_by_a_finished_rebuild() {
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
    let persisted = fingerprint(
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    );

    // A previous rebuild of the same identity recorded and then finished its
    // target. The barrier is idle and keeps no target.
    let first = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin first rebuild");
    db.begin_rebuild_in(&mut tx, first.item_id, first.token, &persisted, COLLECTION)
        .await
        .expect("start the first rebuild");
    tx.commit().await.expect("commit the first rebuild");
    let mut tx = db.pool().begin().await.expect("begin first completion");
    db.complete_rebuild_in(&mut tx, first.item_id, first.token)
        .await
        .expect("clear the first barrier");
    tx.commit().await.expect("commit the first completion");
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Idle);
    // The finished rebuild leaves no active rebuild task behind: the
    // deployment admits one active vector rebuild at a time.
    sqlx::query("UPDATE context69.tasks SET status = 'succeeded' WHERE id = $1")
        .bind(first.task_id)
        .execute(db.pool())
        .await
        .expect("finish the first rebuild task");

    // The claim alone decides: an idle barrier creates the fresh target even
    // when a stale pre-read would still remember the previous collection.
    let second = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin fresh claim");
    let started = db
        .begin_rebuild_checked_in(
            &mut tx,
            second.item_id,
            second.token,
            &persisted,
            "fresh-target",
        )
        .await
        .expect("claim the idle barrier");
    tx.commit().await.expect("commit the fresh claim");

    let (started, resumed) = started;
    match started {
        RebuildStartOutcome::Started(barrier) => {
            assert!(!resumed, "an idle claim never resumes a target");
            assert_eq!(
                barrier.collection.as_deref(),
                Some("fresh-target"),
                "the idle claim records the fresh target"
            );
        }
        other => panic!("the idle barrier must start a fresh target: {other:?}"),
    }

    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_reclaimed_paused_owner_stays_parked_and_resumable() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let seeded = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        seeded.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");
    let mut tx = db.pool().begin().await.expect("begin pause request");
    assert!(
        db.request_rebuild_pause_in(&mut tx, seeded.item_id, seeded.token)
            .await
            .expect("request the pause"),
        "the owner requests the durable pause"
    );
    tx.commit().await.expect("commit the pause request");

    // The process dies before it parks the item. A reclaim hands the same item
    // a fresh token, but the paused barrier still names that item as its owner.
    set_item_lease(&db, seeded.item_id, Uuid::new_v4(), false).await;
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert_eq!(barrier.state, RebuildBarrierState::Running);
    assert!(barrier.pause_requested);
    assert_eq!(
        barrier.owner_item_id,
        Some(seeded.item_id),
        "the paused owner is the item, not the dead attempt token"
    );

    // The recovered item parks on the exact paused dependency the explicit
    // resume reopens, and the resume keeps the durable cursor.
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', waiting_reason = 'dependency', \
             dependency_key = 'vector_rebuild_paused', lease_token = NULL, \
             lease_until = NULL, finished_at = NULL \
         WHERE id = $1",
    )
    .bind(seeded.item_id)
    .execute(db.pool())
    .await
    .expect("park the recovered item");
    assert!(
        db.resume_paused_rebuild_item(seeded.task_id)
            .await
            .expect("resume the paused item"),
        "the explicit resume reopens the parked owner"
    );
    let barrier = db
        .read_rebuild_barrier()
        .await
        .expect("read the resumed barrier");
    assert!(
        !barrier.pause_requested,
        "the explicit resume clears the pause"
    );
    assert_eq!(barrier.owner_item_id, Some(seeded.item_id));

    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancel_that_lost_to_a_committed_activation_reports_too_late() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let seeded = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        seeded.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // The activation transaction commits the cleared barrier and the finished
    // owner item together; the parent recompute follows on the worker path.
    let mut tx = db.pool().begin().await.expect("begin activation");
    db.complete_rebuild_in(&mut tx, seeded.item_id, seeded.token)
        .await
        .expect("clear the barrier");
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'succeeded', stage = 'finalize', lease_token = NULL, \
             lease_until = NULL, finished_at = now() \
         WHERE id = $1",
    )
    .bind(seeded.item_id)
    .execute(&mut *tx)
    .await
    .expect("finish the activated item");
    sqlx::query_file!("src/sql/db/tasks/recompute.sql", seeded.task_id)
        .execute(&mut *tx)
        .await
        .expect("project the activated parent in the same transaction");
    tx.commit().await.expect("commit the activation");

    assert!(
        !db.cancel_task(seeded.task_id, seeded.user_id)
            .await
            .expect("cancel the already activated task"),
        "a cancel that lost to the committed activation reports too late"
    );
    let task = db
        .get_task_internal(seeded.task_id)
        .await
        .expect("read the task")
        .expect("the task exists");
    assert_eq!(
        task.status, "succeeded",
        "the committed activation is never mutated into a cancelled task"
    );

    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn bulk_cancel_never_turns_a_committed_activation_into_cancelled() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    let seeded = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        seeded.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // Commit the exact activation state in one transaction: cleared barrier,
    // succeeded owner item and recomputed parent.
    let mut tx = db.pool().begin().await.expect("begin activation");
    db.complete_rebuild_in(&mut tx, seeded.item_id, seeded.token)
        .await
        .expect("clear the barrier");
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'succeeded', stage = 'finalize', lease_token = NULL, \
             lease_until = NULL, finished_at = now() \
         WHERE id = $1",
    )
    .bind(seeded.item_id)
    .execute(&mut *tx)
    .await
    .expect("finish the activated item");
    sqlx::query_file!("src/sql/db/tasks/recompute.sql", seeded.task_id)
        .execute(&mut *tx)
        .await
        .expect("project the activated parent");
    tx.commit().await.expect("commit the activation");

    db.cancel_all_active_tasks()
        .await
        .expect("bulk cancellation must not fail on a completed activation");
    let task = db
        .get_task_internal(seeded.task_id)
        .await
        .expect("read the task")
        .expect("the task exists");
    assert_eq!(
        task.status, "succeeded",
        "bulk cancellation never turns a committed activation into cancelled"
    );

    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn generic_and_bulk_cancellation_finish_under_one_lock_order() {
    let db = connect().await;
    reset_barrier(&db).await;

    let seeded = seed_task_item_full(&db, "running", "file_batch").await;

    // Both paths take item -> epoch -> task. Running them at the same time
    // proves neither can hold the task while waiting for the item (the R1
    // deadlock) and that the winner leaves one terminal state.
    let (single, bulk) = tokio::join!(
        db.cancel_task(seeded.task_id, seeded.user_id),
        db.cancel_all_active_tasks()
    );
    single.expect("the single cancellation must not deadlock or fail");
    bulk.expect("the bulk cancellation must not deadlock or fail");

    let task = db
        .get_task_internal(seeded.task_id)
        .await
        .expect("read the task")
        .expect("the task exists");
    assert_eq!(task.status, "cancelled");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn paused_resume_and_bulk_cancellation_finish_under_one_lock_order() {
    let db = connect().await;
    reset_barrier(&db).await;

    let seeded = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        seeded.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");
    let mut tx = db.pool().begin().await.expect("begin pause request");
    db.request_rebuild_pause_in(&mut tx, seeded.item_id, seeded.token)
        .await
        .expect("request the pause");
    tx.commit().await.expect("commit the pause request");
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', waiting_reason = 'dependency', \
             dependency_key = 'vector_rebuild_paused', lease_token = NULL, \
             lease_until = NULL \
         WHERE id = $1",
    )
    .bind(seeded.item_id)
    .execute(db.pool())
    .await
    .expect("park the paused item");

    let (resumed, bulk) = tokio::join!(
        db.resume_paused_rebuild_item(seeded.task_id),
        db.cancel_all_active_tasks()
    );
    resumed.expect("the paused resume must not deadlock or fail");
    bulk.expect("the bulk cancellation must not deadlock or fail");

    let task = db
        .get_task_internal(seeded.task_id)
        .await
        .expect("read the task")
        .expect("the task exists");
    assert_eq!(task.status, "cancelled");

    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_target_ownership_marker_follows_the_attempt_that_created_the_target() {
    let db = connect().await;
    reset_barrier(&db).await;

    let seeded = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        seeded.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // A fresh claim names a target that no attempt has created yet.
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(
        !barrier.rebuild_target_created,
        "a fresh target is not owned before it is created"
    );

    // The marker is written under the exact owner: the owner item, its grant
    // token, the captured fingerprint and the recorded collection.
    let mut tx = db.pool().begin().await.expect("begin the mark");
    let marked = db
        .mark_rebuild_target_created_in(
            &mut tx,
            seeded.item_id,
            seeded.token,
            FINGERPRINT,
            COLLECTION,
        )
        .await
        .expect("mark the created target");
    assert!(marked, "the owner marks the target it created");
    tx.commit().await.expect("commit the mark");
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(
        barrier.rebuild_target_created,
        "the recorded target is owned by this attempt"
    );

    // A foreign owner marks nothing, and a foreign fingerprint marks nothing.
    let other = seed_task_item_full(&db, "running", "file_batch").await;
    let mut tx = db.pool().begin().await.expect("begin the foreign mark");
    let marked = db
        .mark_rebuild_target_created_in(
            &mut tx,
            other.item_id,
            other.token,
            FINGERPRINT,
            COLLECTION,
        )
        .await
        .expect("mark the created target");
    assert!(!marked, "a foreign owner marks nothing");
    let marked = db
        .mark_rebuild_target_created_in(
            &mut tx,
            seeded.item_id,
            seeded.token,
            "another-fingerprint",
            COLLECTION,
        )
        .await
        .expect("mark the created target");
    assert!(!marked, "another identity marks nothing");
    tx.rollback().await.expect("roll back the foreign marks");

    // An owned takeover of the same identity keeps the marker: the recorded
    // target is still the collection that attempt created. The previous owner
    // is superseded by a reclaim, so its grant is gone.
    let reclaimed_token = Uuid::new_v4();
    set_item_lease(&db, seeded.item_id, reclaimed_token, false).await;
    let mut tx = db.pool().begin().await.expect("begin the takeover");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        reclaimed_token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("take the barrier over");
    tx.commit().await.expect("commit the takeover");
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(
        barrier.rebuild_target_created,
        "an owned resume keeps the ownership of the recorded target"
    );

    // A released target is owned by nobody any more.
    let mut tx = db.pool().begin().await.expect("begin the cancel");
    db.cancel_owned_rebuild_in(&mut tx, Some(seeded.task_id), false)
        .await
        .expect("free the cancelled barrier");
    tx.commit().await.expect("commit the cancel");
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(
        !barrier.rebuild_target_created,
        "a released target is owned by nobody"
    );

    delete_tasks(&db, &[other.task_id]).await;
    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_refused_target_frees_only_the_barrier_of_its_own_owner() {
    let db = connect().await;
    reset_barrier(&db).await;

    let first = seed_task_item_full(&db, "running", "file_batch").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, first.item_id, first.token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // A newer owner takes the barrier over: the previous owner is not a live
    // writer any more, because its grant was superseded.
    let reclaimed_token = Uuid::new_v4();
    set_item_lease(&db, first.item_id, reclaimed_token, false).await;
    let second = seed_task_item_full(&db, "running", "file_batch").await;
    let mut tx = db.pool().begin().await.expect("begin the takeover");
    db.begin_rebuild_in(
        &mut tx,
        second.item_id,
        second.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("take the barrier over");
    tx.commit().await.expect("commit the takeover");

    // The refused owner frees nothing: the barrier belongs to the newer
    // owner, and its recorded target is not the collection of the refuser.
    let mut tx = db.pool().begin().await.expect("begin the refused clear");
    let cleared = db
        .clear_owned_rebuild_target_in(&mut tx, first.item_id, first.token, COLLECTION)
        .await
        .expect("clear the refused target");
    assert!(
        cleared.is_none(),
        "a refused owner never frees the barrier of a newer owner"
    );
    tx.rollback().await.expect("roll back the refused clear");
    assert_eq!(
        barrier_state(&db).await,
        RebuildBarrierState::Running,
        "the barrier of the newer owner stays"
    );

    // The owner of the barrier frees its own barrier, and the recorded target
    // is released with it.
    let mut tx = db.pool().begin().await.expect("begin the clear");
    let cleared = db
        .clear_owned_rebuild_target_in(&mut tx, second.item_id, second.token, COLLECTION)
        .await
        .expect("clear the refused target");
    assert!(cleared.is_some(), "the owner frees its own barrier");
    tx.commit().await.expect("commit the clear");
    assert_eq!(
        barrier_state(&db).await,
        RebuildBarrierState::Idle,
        "the released barrier is idle"
    );
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(
        !barrier.rebuild_target_created,
        "a released target is owned by nobody"
    );

    delete_tasks(&db, &[first.task_id, second.task_id]).await;
    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_bulk_cancellation_covers_only_the_segments_of_the_items_it_cancelled() {
    let db = connect().await;
    reset_barrier(&db).await;

    // The active task of this case is cancelled by the bulk cancellation.
    let cancelled = seed_task_item_full(&db, "queued", "file_batch").await;
    let cancelled_segment = db
        .produce_item_segment(cancelled.item_id)
        .await
        .expect("produce the segment of the cancelled item");
    // The other task is already terminal: its item keeps its own segment,
    // because the bulk cancellation never covered that item.
    let untouched = seed_task_item_full(&db, "queued", "file_batch").await;
    sqlx::query("UPDATE context69.tasks SET status = 'succeeded' WHERE id = $1")
        .bind(untouched.task_id)
        .execute(db.pool())
        .await
        .expect("finish the other task");
    let untouched_segment = db
        .produce_item_segment(untouched.item_id)
        .await
        .expect("produce the segment of the untouched item");

    // A paused rebuild owner that arrived after the bulk candidate snapshot:
    // its task is terminal, so the bulk cancellation never covers its item and
    // its durable cursor must survive.
    let paused = seed_task_item_full(&db, "waiting", "vector_rebuild").await;
    sqlx::query("UPDATE context69.tasks SET status = 'succeeded' WHERE id = $1")
        .bind(paused.task_id)
        .execute(db.pool())
        .await
        .expect("finish the paused owner's task");
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_state = 'running', rebuild_owner_item = $1, \
             rebuild_owner_token = $2, rebuild_fingerprint = 'paused-fingerprint', \
             rebuild_collection = 'paused-target', rebuild_started_at = now(), \
             rebuild_pause_requested = TRUE, rebuild_cursor_document_id = 7, \
             rebuild_cursor_chunk_index = 3, rebuild_processed_chunks = 11, \
             rebuild_total_chunks = 20, rebuild_target_created = TRUE \
         WHERE id = 1",
    )
    .bind(paused.item_id)
    .bind(paused.token)
    .execute(db.pool())
    .await
    .expect("record the paused rebuild owner");

    let cancelled_tasks = db
        .cancel_all_active_tasks()
        .await
        .expect("cancel the active tasks");
    assert!(
        cancelled_tasks >= 1,
        "the bulk cancellation cancels the active task"
    );

    let barrier = db
        .read_rebuild_barrier()
        .await
        .expect("read the paused barrier after the bulk cancellation");
    assert_eq!(
        barrier.owner_item_id,
        Some(paused.item_id),
        "the bulk cancellation must not clear an owner outside its captured set"
    );
    assert_eq!(
        barrier.cursor,
        Some(RebuildCursor {
            document_id: 7,
            chunk_index: 3,
        }),
        "the paused owner keeps its cursor"
    );
    assert_eq!(barrier.processed_chunks, 11);
    assert!(barrier.pause_requested);

    let cancelled_status: String =
        sqlx::query_scalar("SELECT status FROM cloudiful_tasks.tasks WHERE id = $1")
            .bind(cancelled_segment)
            .fetch_one(db.pool())
            .await
            .expect("read the cancelled segment");
    assert_eq!(
        cancelled_status, "cancelled",
        "the segment of a cancelled item is cancelled"
    );
    let untouched_status: String =
        sqlx::query_scalar("SELECT status FROM cloudiful_tasks.tasks WHERE id = $1")
            .bind(untouched_segment)
            .fetch_one(db.pool())
            .await
            .expect("read the untouched segment");
    assert_eq!(
        untouched_status, "queued",
        "the segment of an item the cancellation did not cover is untouched"
    );
    let untouched_item: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(untouched.item_id)
            .fetch_one(db.pool())
            .await
            .expect("read the untouched item");
    assert_eq!(
        untouched_item, "queued",
        "the item of an already terminal task keeps its status"
    );

    delete_tasks(&db, &[cancelled.task_id, untouched.task_id, paused.task_id]).await;
    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancelled_or_discarded_rebuild_frees_the_barrier_and_a_stale_completion_writes_nothing()
{
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;

    // A running barrier is freed by the generic task cancellation.
    let seeded = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        seeded.item_id,
        seeded.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");
    let mut connection = db.pool().acquire().await.expect("acquire");
    assert!(
        db.advance_rebuild_cursor_in(
            &mut connection,
            seeded.item_id,
            seeded.token,
            FINGERPRINT,
            COLLECTION,
            context69::db::RebuildCheckpoint {
                cursor: RebuildCursor {
                    document_id: 2,
                    chunk_index: 1,
                },
                processed_chunks: 4,
                total_chunks: 8,
            },
        )
        .await
        .expect("advance the cursor"),
        "the owner advances its cursor"
    );
    drop(connection);
    let epoch_before = db.read_search_registry_epoch().await.expect("epoch before");

    assert!(
        db.cancel_task(seeded.task_id, seeded.user_id)
            .await
            .expect("cancel the rebuild task"),
        "the owner cancels its rebuild task"
    );
    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert_eq!(barrier.state, RebuildBarrierState::Idle);
    assert_eq!(barrier.owner_item_id, None);
    assert_eq!(barrier.cursor, None);
    assert_eq!(barrier.processed_chunks, 0);
    assert_eq!(barrier.total_chunks, 0);
    assert_eq!(
        db.read_search_registry_epoch().await.expect("epoch after"),
        epoch_before + 1,
        "cancellation bumps the epoch exactly once"
    );

    // A late completion of the cancelled attempt writes nothing.
    let mut tx = db.pool().begin().await.expect("begin stale completion");
    let cleared = db
        .complete_owned_rebuild_in(
            &mut tx,
            seeded.item_id,
            seeded.token,
            &context69::db::VectorIndexState {
                collection_name: COLLECTION,
                fingerprint: FINGERPRINT,
                embedding_base_url: "https://embeddings.example/v1",
                embedding_model: "model-a",
                dimensions: 8,
                rebuilt_chunks: 8,
            },
        )
        .await
        .expect("try the stale completion");
    assert_eq!(cleared, None, "a cancelled owner never completes");
    tx.rollback().await.expect("roll back the stale completion");
    assert_eq!(
        db.get_vector_index_fingerprint(COLLECTION)
            .await
            .expect("read the identity"),
        None,
        "a cancelled rebuild never writes an identity row"
    );

    // A failed barrier keeps blocking editing until it is explicitly discarded;
    // discarding it does not reopen the terminal task.
    let failed = seed_task_item_full(&db, "failed", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin failed rebuild");
    db.begin_rebuild_in(
        &mut tx,
        failed.item_id,
        failed.token,
        FINGERPRINT,
        COLLECTION,
    )
    .await
    .expect("start the failed rebuild");
    tx.commit().await.expect("commit the rebuild start");
    let mut tx = db.pool().begin().await.expect("begin failure");
    assert!(
        db.fail_rebuild_in(&mut tx, failed.item_id, failed.token, "boom")
            .await
            .expect("record the failure")
    );
    tx.commit().await.expect("commit the failure");
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Failed);

    assert!(
        db.discard_failed_rebuild(failed.task_id)
            .await
            .expect("discard the failed rebuild"),
        "the failed owner is discarded"
    );
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Idle);
    let task = db
        .get_task_internal(failed.task_id)
        .await
        .expect("read the terminal task")
        .expect("the terminal task exists");
    assert_eq!(
        task.status, "failed",
        "the discard never reopens the old failed task"
    );

    reset_barrier(&db).await;
}

/// An ordinary point-writing admission needs exactly one active vector index
/// that still matches the desired configuration.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_active_vector_index_must_exist_and_match_the_desired_configuration() {
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    let identity = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;

    // A deployment without an active pointer fails closed even when its
    // desired configuration is known.
    sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
        .execute(db.pool())
        .await
        .expect("clear the active index");
    let mut tx = db.pool().begin().await.expect("begin admission");
    let refused = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(Uuid::new_v4()),
            Some(&identity),
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await;
    assert!(refused.is_err(), "no active index refuses the write");
    tx.rollback()
        .await
        .expect("roll back the refused admission");

    // The same identity is admitted once its physical collection is active,
    // which proves the desired logical fingerprint and the physical collection
    // are validated together.
    activate_index(
        &db,
        COLLECTION,
        &identity.embedding_fingerprint,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;
    let mut tx = db.pool().begin().await.expect("begin admission");
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(Uuid::new_v4()),
            Some(&identity),
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("admit the staged entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    tx.commit().await.expect("commit the admission");

    // A desired configuration that moved while the old collection stayed active
    // refuses the write instead of publishing into the wrong space.
    seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-b",
        8,
    )
    .await;
    activate_index(
        &db,
        COLLECTION,
        &identity.embedding_fingerprint,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;
    let mut tx = db.pool().begin().await.expect("begin admission");
    let refused = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(Uuid::new_v4()),
            Some(&identity),
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await;
    assert!(
        refused.is_err(),
        "a desired configuration that differs from the active index refuses the write"
    );
    tx.rollback()
        .await
        .expect("roll back the refused admission");

    // The active pointer is unique: a second row cannot become active. The
    // seeding statement activates its row, so inserting another physical
    // collection while one is already active must be refused by the index.
    let second_active = sqlx::query_file!(
        "tests/sql/vector_registry/seed_active_index.sql",
        "other-collection",
        &identity.embedding_fingerprint,
        "https://embeddings.example/v1",
        "model-a",
        8_i64
    )
    .execute(db.pool())
    .await;
    assert!(
        second_active.is_err(),
        "a second active vector index must be refused"
    );

    clear_runtime_settings(&db).await;
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
}

/// A completed rebuild activates the exact target it verified: the previous
/// active collection stops being active, and the target becomes the one
/// collection ordinary point writers may use. A completion that no longer owns
/// the barrier writes nothing at all, so a cancelled build can never switch the
/// active pointer.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_completed_rebuild_activates_the_exact_target() {
    const TARGET: &str = "barrier-collection__target";
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    let desired = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        owner,
        owner_token,
        &desired.embedding_fingerprint,
        TARGET,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // A completion that no longer owns the barrier writes nothing: the
    // previous collection stays active and the target is never published.
    let mut tx = db.pool().begin().await.expect("begin the stale completion");
    let stale = db
        .complete_owned_rebuild_in(
            &mut tx,
            owner,
            Uuid::new_v4(),
            &context69::db::VectorIndexState {
                collection_name: TARGET,
                fingerprint: &desired.embedding_fingerprint,
                embedding_base_url: "https://embeddings.example/v1",
                embedding_model: "model-a",
                dimensions: 8,
                rebuilt_chunks: 12,
            },
        )
        .await
        .expect("the stale completion is answered");
    assert!(stale.is_none(), "a stale owner writes nothing");
    tx.rollback().await.expect("roll back the stale completion");
    let still_active = db
        .read_active_vector_index_state()
        .await
        .expect("read the active vector index")
        .expect("the previous collection is still active");
    assert_eq!(still_active.collection_name, COLLECTION);

    // The owning completion records the target and activates exactly it.
    let mut tx = db.pool().begin().await.expect("begin the completion");
    let cleared = db
        .complete_owned_rebuild_in(
            &mut tx,
            owner,
            owner_token,
            &context69::db::VectorIndexState {
                collection_name: TARGET,
                fingerprint: &desired.embedding_fingerprint,
                embedding_base_url: "https://embeddings.example/v1",
                embedding_model: "model-a",
                dimensions: 8,
                rebuilt_chunks: 12,
            },
        )
        .await
        .expect("the owning completion is answered");
    assert!(cleared.is_some(), "the owner clears the barrier");
    tx.commit().await.expect("commit the completion");

    let active = db
        .read_active_vector_index_state()
        .await
        .expect("read the active vector index")
        .expect("the target is the active collection");
    assert_eq!(active.collection_name, TARGET);
    assert_eq!(active.fingerprint, desired.embedding_fingerprint);
    assert_eq!(active.dimensions, 8);

    // The previous collection is still recorded, but it is no longer active.
    let previous: Option<(String, bool)> = sqlx::query_as(
        "SELECT collection_name, active FROM context69.vector_index_state \
         WHERE collection_name = $1",
    )
    .bind(COLLECTION)
    .fetch_optional(db.pool())
    .await
    .expect("read the previous collection");
    assert_eq!(
        previous,
        Some((COLLECTION.to_string(), false)),
        "the previous collection stops being active"
    );

    sqlx::query("DELETE FROM context69.vector_index_state WHERE collection_name = ANY($1)")
        .bind(&[COLLECTION.to_string(), TARGET.to_string()])
        .execute(db.pool())
        .await
        .expect("clear the recorded identities");
    clear_runtime_settings(&db).await;
    // The target owns the schema, so the barrier is restored after the
    // case as well: another suite that has to move the vector space
    // must not meet a rebuild this target left behind.
    reset_barrier(&db).await;
}

/// The control projection reads the durable rows rather than a process memory:
/// the task that owns the barrier first, then the newer rebuild task, and the
/// one active vector index even while no rebuild task exists.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_rebuild_control_projection_prefers_the_barrier_owner_and_reports_the_active_index() {
    const TARGET: &str = "barrier-collection__control-target";
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    let desired = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;

    // Without a rebuild task the active index is still reported.
    let rows = db
        .read_vector_rebuild_control_rows()
        .await
        .expect("read the control rows");
    assert!(!rows.is_empty(), "the base row reports the active index");
    assert_eq!(rows[0].active_collection_name.as_deref(), Some(COLLECTION));
    assert_eq!(rows[0].active_model.as_deref(), Some("model-a"));
    assert_eq!(rows[0].active_dimensions, Some(8));

    let owner = seed_task_item_full(&db, "running", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        owner.item_id,
        owner.token,
        &desired.embedding_fingerprint,
        TARGET,
    )
    .await
    .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");

    // A parked item records the dependency that makes it a paused rebuild.
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', dependency_key = 'vector_rebuild_paused' \
         WHERE id = $1",
    )
    .bind(owner.item_id)
    .execute(db.pool())
    .await
    .expect("park the paused item");
    let rows = db
        .read_vector_rebuild_control_rows()
        .await
        .expect("read the control rows");
    assert_eq!(rows[0].id, Some(owner.task_id));
    assert_eq!(rows[0].item_id, Some(owner.item_id));
    assert_eq!(
        rows[0].item_dependency_key.as_deref(),
        Some("vector_rebuild_paused"),
        "the parked dependency is projected"
    );

    // A failed barrier keeps its owner after the task is terminal, so a newer
    // queued rebuild exists at the same time. The failed owner stays first.
    let mut connection = db.pool().acquire().await.expect("acquire");
    assert!(
        db.fail_rebuild_in(
            &mut connection,
            owner.item_id,
            owner.token,
            "the rebuild failed",
        )
        .await
        .expect("fail the rebuild"),
        "the owner fails its barrier"
    );
    sqlx::query("UPDATE context69.tasks SET status = 'failed' WHERE id = $1")
        .bind(owner.task_id)
        .execute(db.pool())
        .await
        .expect("fail the owner task");
    let queued = seed_task_item_full(&db, "queued", "vector_rebuild").await;

    let rows = db
        .read_vector_rebuild_control_rows()
        .await
        .expect("read the control rows");
    assert_eq!(
        rows[0].id,
        Some(owner.task_id),
        "the failed barrier owner is ordered before a newer queued rebuild"
    );
    assert_eq!(rows[0].status.as_deref(), Some("failed"));
    assert_eq!(
        rows[0].error_summary, None,
        "the failed barrier keeps its error in the barrier row, not in the task"
    );
    let queued_row = rows
        .iter()
        .find(|row| row.id == Some(queued.task_id))
        .expect("the queued rebuild is reported");
    assert_eq!(queued_row.status.as_deref(), Some("queued"));

    sqlx::query("DELETE FROM context69.vector_index_state WHERE collection_name = $1")
        .bind(COLLECTION)
        .execute(db.pool())
        .await
        .expect("clear the recorded identity");
    clear_runtime_settings(&db).await;
    reset_barrier(&db).await;
}
