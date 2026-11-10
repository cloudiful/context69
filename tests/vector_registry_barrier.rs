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

#[cfg(feature = "integration-test-helpers")]
#[path = "vector_registry_barrier/http_mock.rs"]
mod http_mock;
#[path = "vector_registry_barrier/support.rs"]
mod support;

use context69::db::{
    AdmissionOutcome, RebuildBarrierState, RebuildCursor, RebuildRefusalReason,
    RebuildStartOutcome, RegistryQuotaOverride,
};
use uuid::Uuid;

#[cfg(feature = "integration-test-helpers")]
use std::sync::Arc;

#[cfg(feature = "integration-test-helpers")]
use anyhow::{Context, Result};
#[cfg(feature = "integration-test-helpers")]
use async_trait::async_trait;
#[cfg(feature = "integration-test-helpers")]
use cloudiful_tasks::{AttemptStatus, TaskId, TaskPauseState};
#[cfg(feature = "integration-test-helpers")]
use context69::db::{CreateTaskSubmissionRequest, Database};
#[cfg(feature = "integration-test-helpers")]
use context69::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    services::{
        document_store::DocumentStoreService,
        library::{LibraryService, LibraryServiceConfig},
        namespace::NamespaceService,
        secret_store,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::{TaskService, TaskServiceDependencies},
        translation::TranslationPublisherAdapter,
    },
};
#[cfg(feature = "integration-test-helpers")]
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
#[cfg(feature = "integration-test-helpers")]
use serde_json::json;

use support::{
    activate_index, barrier_state, clear_registry_entries, clear_runtime_settings, connect,
    delete_registry_entries, delete_tasks, fingerprint, identity, register_registry_entry,
    registry_entry, registry_entry_exists, reset_barrier, seed_runtime_settings, seed_task_item,
    seed_task_item_full, set_item_lease,
};

const FINGERPRINT: &str = "barrier-fingerprint";
const COLLECTION: &str = "barrier-collection";

/// Readiness only: the vector checkpoint never runs translation.
#[cfg(feature = "integration-test-helpers")]
struct NotReady;

#[cfg(feature = "integration-test-helpers")]
#[async_trait]
impl TranslationReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// The production task service graph with no network runtimes, so the real
/// runtime driver can park a paused item without reaching Qdrant or embeddings.
#[cfg(feature = "integration-test-helpers")]
async fn build_task_service(db: &Database) -> anyhow::Result<TaskService> {
    let chunking = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(TranslationPublisherAdapter::new(
            None,
            None,
            chunking.clone(),
        )),
        concurrency: 1,
        readiness: Arc::new(NotReady),
    });
    let storage_root = std::env::temp_dir().join(format!("context69-vector-{}", Uuid::new_v4()));
    let library = LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root,
                max_upload_size_mb: 1,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "vector-test".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
    )
    .await
    .context("build the library service")?;
    let sync = SyncService::new(
        db.clone(),
        secret_store::build_unkeyed(db),
        None,
        None,
        chunking,
        1,
        translation.clone(),
    );
    Ok(TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: NamespaceService::new(db.clone()),
        document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
        library: library.clone(),
        sync: sync.clone(),
        source_folders: SourceFoldersService::new(db.clone(), library, sync),
        translation,
        secrets: context69::services::secret_store::build_unkeyed(&db),
        concurrency: 1,
    }))
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unresolved_staging_entry_blocks_the_rebuild_start() {
    let db = connect().await;
    reset_barrier(&db).await;
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
    // The staged entry is this case's own fixture: it is cleared by its exact
    // run id, so an unresolved run of this target never blocks another suite
    // that has to move the vector space.
    delete_registry_entries(&db, &[entry.run_id]).await;
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

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");
    support::pin_rebuild_snapshot(&db, &pinned_budget(), 1).await;

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
    support::pin_rebuild_snapshot(&db, &pinned_budget(), 1).await;

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

    delete_registry_entries(&db, &[entry.run_id]).await;
    clear_runtime_settings(&db).await;
    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_readiness_of_the_deployment_observes_the_barrier() {
    let db = connect().await;
    reset_barrier(&db).await;
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
    support::pin_rebuild_snapshot(&db, &pinned_budget(), 1).await;
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
    delete_registry_entries(&db, &[entry.run_id]).await;
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

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(&mut tx, owner, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("start the rebuild");
    tx.commit().await.expect("commit the rebuild start");
    support::pin_rebuild_snapshot(&db, &pinned_budget(), 1).await;
    support::mark_rebuild_staging_complete(&db).await;

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
    support::pin_rebuild_snapshot(&db, &pinned_budget(), 1).await;

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
    let identity = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;

    // A different fixture owns this sentinel entry: teardown must remove only
    // this case's own run ids, never every registry row in the shared database.
    let foreign = registry_entry(Uuid::new_v4());
    register_registry_entry(&db, &foreign).await;

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
    let admitted_entry = registry_entry(Uuid::new_v4());
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &admitted_entry,
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

    delete_registry_entries(&db, &[admitted_entry.run_id]).await;
    assert!(
        registry_entry_exists(&db, foreign.run_id).await,
        "exact teardown preserves another fixture's registry entry"
    );
    delete_registry_entries(&db, &[foreign.run_id]).await;
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
    support::pin_rebuild_snapshot(&db, &pinned_budget(), 1).await;
    support::mark_rebuild_staging_complete(&db).await;

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

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_common_pause_sets_the_barrier_flag_and_keeps_the_cursor() {
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
        .expect("advance the cursor")
    );
    drop(connection);

    assert!(
        db.request_task_pause(seeded.task_id, seeded.user_id, true)
            .await
            .expect("pause the rebuild task"),
        "the common primitive pauses the running rebuild"
    );

    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(barrier.pause_requested);
    assert_eq!(
        barrier.cursor,
        Some(RebuildCursor {
            document_id: 7,
            chunk_index: 3,
        }),
        "the common pause keeps the committed cursor"
    );
    assert_eq!(barrier.processed_chunks, 2);
    assert_eq!(barrier.total_chunks, 9);
    assert_eq!(barrier.owner_item_id, Some(seeded.item_id));
    assert_eq!(barrier.owner_token, Some(seeded.token));

    let projection = sqlx::query_file!(
        "tests/sql/task_execution/read_pause_projection.sql",
        seeded.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the paused task projection");
    assert_eq!(projection.pause_state, "pausing");

    reset_barrier(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_common_resume_clears_the_barrier_flag_without_moving_the_cursor() {
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
        .expect("advance the cursor")
    );
    drop(connection);

    assert!(
        db.request_task_pause(seeded.task_id, seeded.user_id, true)
            .await
            .expect("pause the rebuild task")
    );
    assert!(
        db.request_task_resume(seeded.task_id, seeded.user_id, true)
            .await
            .expect("resume the rebuild task"),
        "the common primitive resumes the paused rebuild"
    );

    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(!barrier.pause_requested);
    assert_eq!(
        barrier.cursor,
        Some(RebuildCursor {
            document_id: 7,
            chunk_index: 3,
        }),
        "the common resume never moves the committed cursor"
    );
    assert_eq!(barrier.processed_chunks, 2);
    assert_eq!(barrier.total_chunks, 9);
    assert_eq!(barrier.owner_item_id, Some(seeded.item_id));
    assert_eq!(barrier.owner_token, Some(seeded.token));

    let projection = sqlx::query_file!(
        "tests/sql/task_execution/read_pause_projection.sql",
        seeded.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the resumed task projection");
    assert_eq!(projection.pause_state, "unpaused");

    reset_barrier(&db).await;
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_running_vector_checkpoint_parks_through_the_driver_and_resumes_immediately() {
    let db = connect().await;
    reset_barrier(&db).await;
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'not-a-real-hash') RETURNING id",
    )
    .bind(format!("vector-checkpoint-{}", Uuid::new_v4()))
    .bind("vector checkpoint owner")
    .fetch_one(db.pool())
    .await
    .expect("seed user");
    let task_id = Uuid::new_v4();
    let (_, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "vector_rebuild",
            group_path: Some("test/vector-checkpoint"),
            source_key: None,
            payloads: &[json!({})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("vector-checkpoint-{}", Uuid::new_v4()),
        })
        .await
        .expect("create the rebuild task");
    let item_id = item_ids[0];
    db.produce_item_segment(item_id)
        .await
        .expect("produce the segment");
    let claimed = db
        .claim_items_fast(10)
        .await
        .expect("claim the rebuild")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("the rebuild item is claimed");
    let execution = claimed
        .execution_context()
        .expect("the claimed item carries its captured execution");

    let mut tx = db.pool().begin().await.expect("begin rebuild");
    db.begin_rebuild_in(
        &mut tx,
        item_id,
        claimed.lease_token,
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
            item_id,
            claimed.lease_token,
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
        .expect("advance the cursor")
    );
    drop(connection);

    assert!(
        db.request_task_pause(task_id, user_id, true)
            .await
            .expect("request the pause"),
        "the common primitive pauses the running rebuild"
    );
    let service = build_task_service(&db)
        .await
        .expect("build the task service");
    service
        .run_item_for_test(claimed.clone())
        .await
        .expect("the runtime driver consumes the paused checkpoint");

    let native_attempt: String =
        sqlx::query_scalar("SELECT status FROM cloudiful_tasks.attempts WHERE id = $1")
            .bind(execution.attempt_id())
            .fetch_one(db.pool())
            .await
            .expect("read the native attempt");
    assert_eq!(native_attempt, "paused");
    let domain_attempt =
        sqlx::query_file!("tests/sql/task_execution/read_pause_attempt.sql", item_id)
            .fetch_one(db.pool())
            .await
            .expect("read the domain attempt");
    assert_eq!(domain_attempt.status, "paused");

    let projection = sqlx::query_file!(
        "tests/sql/task_execution/read_pause_projection.sql",
        item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the parked projection");
    assert_eq!(projection.pause_state, "paused");
    assert_eq!(projection.item_status, "queued");
    assert_eq!(
        projection.execution_task_id,
        Some(execution.task_id()),
        "the parked item keeps its exact segment"
    );
    assert!(projection.next_attempt_at.is_none());
    assert_eq!(
        projection.paused_attempts, 1,
        "the park adds exactly one parked attempt"
    );

    let barrier = db.read_rebuild_barrier().await.expect("read the barrier");
    assert!(barrier.pause_requested);
    assert_eq!(barrier.owner_item_id, Some(item_id));
    assert_eq!(barrier.owner_token, Some(claimed.lease_token));
    assert_eq!(
        barrier.cursor,
        Some(RebuildCursor {
            document_id: 7,
            chunk_index: 3,
        }),
        "the pause keeps the committed cursor"
    );
    assert_eq!(barrier.processed_chunks, 2);
    assert_eq!(barrier.total_chunks, 9);

    assert!(
        db.request_task_resume(task_id, user_id, true)
            .await
            .expect("resume the parked rebuild"),
        "the explicit resume reopens the parked item"
    );
    let barrier = db
        .read_rebuild_barrier()
        .await
        .expect("read the resumed barrier");
    assert!(!barrier.pause_requested);
    let resumed = db
        .claim_items_fast(10)
        .await
        .expect("claim after the resume")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("the resumed rebuild is immediately claimable");
    assert_eq!(
        resumed.execution_generation, claimed.execution_generation,
        "the park and resume never mint a new generation"
    );
    assert_eq!(
        resumed.execution_task_id,
        Some(execution.task_id()),
        "the resumed claim is the same segment"
    );

    reset_barrier(&db).await;
    delete_tasks(&db, &[task_id]).await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("delete the user");
}

#[cfg(feature = "integration-test-helpers")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RebuildCaseFault {
    None,
    ProviderFails,
    PanicAfterCollection,
    DropResumedClaim,
}

#[cfg(feature = "integration-test-helpers")]
enum CaseFailure {
    Returned(anyhow::Error),
    Panicked(String),
}

#[cfg(feature = "integration-test-helpers")]
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Runs one owned-resource operation, turning an unwind into a reported failure
/// so the caller can still dispose every resource it created.
#[cfg(feature = "integration-test-helpers")]
async fn catch_case_failure<F>(future: F) -> Result<(), CaseFailure>
where
    F: std::future::Future<Output = anyhow::Result<()>>,
{
    use futures::FutureExt;
    match std::panic::AssertUnwindSafe(future).catch_unwind().await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(CaseFailure::Returned(error)),
        Err(payload) => Err(CaseFailure::Panicked(panic_message(payload))),
    }
}

#[cfg(feature = "integration-test-helpers")]
async fn run_actual_rebuild_case(db: &Database, fault: RebuildCaseFault) -> anyhow::Result<()> {
    let qdrant_url = "http://unraid.cloud1ful.com:6336";

    // Clear only the fixture-owned rows an earlier case of this target left
    // behind. Fixture tasks and registry entries carry no cross-run identity,
    // and the runner reset this schema for the target, so they are the
    // target's own predecessors. Foreign runtime singletons, vector state and
    // non-fixture rows are never cleared; the strict preflight below still
    // refuses them.
    support::reset_barrier(db).await;

    // Read-only emptiness gate: this case owns the five runtime singletons, the
    // recorded vector index rows, the registry entries and the one durable
    // barrier. A previous failed run or a live peer makes the reusable fixture
    // unsafe, so refuse before any write instead of clearing foreign state.
    let preflight = support::read_rebuild_fixture_preflight(db)
        .await
        .context("read the rebuild fixture preflight")?;
    if let Some(problem) = preflight.emptiness_problem() {
        anyhow::bail!(
            "the reusable rebuild fixture is not empty; refusing before any write: {problem}"
        );
    }
    let existing_collections = support::pause_collections(qdrant_url).await?;
    if !existing_collections.is_empty() {
        anyhow::bail!(
            "the reusable rebuild fixture still owns Qdrant collection(s): {existing_collections:?}"
        );
    }

    let base_collection = format!("context69_issue_21_pause_{}", Uuid::new_v4().simple());

    let mut group_id: Option<i64> = None;
    let mut user_id: Option<i64> = None;
    let mut task_id: Option<Uuid> = None;
    let mut item_id: Option<Uuid> = None;
    let mut mock: Option<http_mock::EmbeddingMock> = None;
    let mut run_handle: Option<tokio::task::JoinHandle<anyhow::Result<()>>> = None;
    let mut owned_collection: Option<String> = None;
    let mut expected_cursor: Option<RebuildCursor> = None;
    let mut total_chunks = 0i64;
    let mut segment: Option<Uuid> = None;
    let mut generation = 0i64;
    // One absolute budget for the whole actual gate: setup, the first data
    // batch, the pause checkpoint and the worker return. The happy path is
    // generous for a remote 4096-dimension page; the failure probe only needs
    // to prove that the wait is bounded and never becomes a retry loop.
    let deadline_seconds = if fault == RebuildCaseFault::ProviderFails {
        5
    } else {
        30
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(deadline_seconds);

    let outcome = catch_case_failure(async {
        context69_db_schema::bootstrap_library_schema(db.pool()).await?;
        let gid: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_group.sql",
            format!("reindex-{}", Uuid::new_v4()),
            "Reindex fixture",
            format!("test/reindex-{}", Uuid::new_v4()),
        )
        .fetch_one(db.pool())
        .await?;
        group_id = Some(gid);
        let (document_id, fresh_chunks) = support::seed_reindex_document(&db, gid, 65)
            .await?;
        let fresh_chunks = i64::try_from(fresh_chunks).context("fresh chunk count")?;

        // The rebuild re-chunks the canonical body; the global page order is
        // just the single seeded document's fresh chunk index.
        expected_cursor = (fresh_chunks >= 64).then_some(RebuildCursor {
            document_id,
            chunk_index: 63,
        });
        total_chunks = fresh_chunks;

        let started_mock = if fault == RebuildCaseFault::ProviderFails {
            http_mock::EmbeddingMock::start_failing_proof(4096).await?
        } else {
            http_mock::EmbeddingMock::start(4096).await?
        };
        let mock_base_url = started_mock.base_url().to_string();
        mock = Some(started_mock);

        support::try_seed_runtime_settings(
            &db,
            qdrant_url,
            &base_collection,
            &mock_base_url,
            "rebuild-test-model",
            4096,
        )
        .await?;

        let uid = support::try_seed_user(&db).await?;
        user_id = Some(uid);
        let tid = support::try_seed_task(&db, uid, "queued", "vector_rebuild").await?;
        task_id = Some(tid);
        let (iid, _token) = support::try_seed_item(&db, tid).await?;
        item_id = Some(iid);
        db.produce_item_segment(iid).await?;
        let claimed = db
            .claim_items_fast(10)
            .await?
            .into_iter()
            .find(|item| item.id == iid)
            .context("the rebuild item is claimed")?;
        let execution = claimed
            .execution_context()
            .context("the claimed item carries its captured execution")?;
        segment = Some(execution.task_id());
        generation = execution.generation;

        let service = build_task_service(&db).await?;
        let run_service = service.clone();
        let run_item = claimed.clone();
        run_handle = Some(tokio::spawn(async move {
            run_service.run_item_for_test(run_item).await
        }));

        // Wait for the actual first data batch while observing the worker and
        // the single deadline: a worker that exits before any data request
        // fails this case promptly instead of leaving the mock Notify pending.
        let mock_ref = mock.as_ref().expect("the embedding mock exists");
        let mut handle = run_handle.take().expect("the rebuild worker handle exists");
        let mut worker_exited = false;
        let gate: anyhow::Result<()> = tokio::select! {
            _ = mock_ref.wait_data_entered() => Ok(()),
            result = &mut handle => {
                worker_exited = true;
                let joined = result.context("join the rebuild worker before its first data batch")?;
                match joined {
                    Ok(()) => Err(anyhow::anyhow!(
                        "the rebuild worker returned successfully before its first data batch"
                    )),
                    Err(error) => Err(anyhow::anyhow!(
                        "the rebuild worker exited before its first data batch: {error:#}"
                    )),
                }
            }
            _ = tokio::time::sleep_until(deadline) => Err(anyhow::anyhow!(
                "the rebuild worker did not reach its first data batch before the {deadline_seconds}s deadline"
            )),
        };
        if !worker_exited {
            run_handle = Some(handle);
        }
        if fault == RebuildCaseFault::ProviderFails {
            // A failing provider never reaches a data batch. The worker may
            // report its own failure or the single deadline may end the wait;
            // either way the gate is bounded and cleanup still runs below.
            anyhow::ensure!(
                gate.is_err(),
                "a failing provider must not reach a data batch"
            );
            anyhow::ensure!(
                mock_ref.proof_requests() >= 1,
                "the failing provider proof reached the mock"
            );
            anyhow::ensure!(
                mock_ref.data_requests() == 0,
                "the failing worker requested no data batch"
            );
            return Ok(());
        }
        gate?;

        let barrier = db.read_rebuild_barrier().await?;
        owned_collection = barrier.collection.clone();
        anyhow::ensure!(
            owned_collection.is_some(),
            "the barrier records the derived rebuild target before the first page"
        );
        anyhow::ensure!(
            barrier.owner_item_id == Some(iid),
            "the running barrier still names the item"
        );
        anyhow::ensure!(
            barrier.owner_token == Some(claimed.lease_token),
            "the running barrier still names the captured token"
        );
        if fault == RebuildCaseFault::PanicAfterCollection {
            panic!("injected post-collection panic after the target was recorded");
        }
        anyhow::ensure!(
            db.request_task_pause(tid, uid, false).await?,
            "the running owner accepts the pause"
        );
        mock_ref.release_data();

        let mut handle = run_handle.take().expect("the rebuild worker handle exists");
        tokio::select! {
            result = &mut handle => {
                result
                    .context("join the paused rebuild worker")??;
            }
            _ = tokio::time::sleep_until(deadline) => {
                run_handle = Some(handle);
                anyhow::bail!(
                    "the paused rebuild worker did not return before the 30s deadline"
                );
            }
        }

        let barrier = db.read_rebuild_barrier().await?;
        anyhow::ensure!(
            barrier.pause_requested,
            "the batch checkpoint observed the pause"
        );
        anyhow::ensure!(
            barrier.owner_item_id == Some(iid) && barrier.owner_token == Some(claimed.lease_token),
            "the parked rebuild keeps its owner"
        );
        anyhow::ensure!(
            barrier.cursor == expected_cursor,
            "the completed first batch is the committed cursor: {:?}",
            barrier.cursor
        );
        anyhow::ensure!(
            barrier.processed_chunks == 64 && barrier.total_chunks == total_chunks,
            "the committed page and the captured total are the checkpoint"
        );

        let queue = db.execution_queue().await?;
        let segment_id = segment.context("the rebuild segment exists")?;
        let native = queue
            .get(TaskId::from_uuid(segment_id))
            .await?
            .context("the native segment still exists")?;
        anyhow::ensure!(
            native.pause_state() == TaskPauseState::Paused,
            "the native task is paused"
        );
        let attempts = queue.attempts(TaskId::from_uuid(segment_id), 10).await?;
        anyhow::ensure!(
            attempts.first().map(cloudiful_tasks::Attempt::status) == Some(AttemptStatus::Paused),
            "the native attempt is sealed paused"
        );

        let projection =
            sqlx::query_file!("tests/sql/task_execution/read_pause_projection.sql", iid)
                .fetch_one(db.pool())
                .await?;
        anyhow::ensure!(
            projection.pause_state == "paused",
            "the parent pause is projected"
        );
        anyhow::ensure!(
            projection.item_status == "queued",
            "the parked item is queued"
        );
        anyhow::ensure!(
            projection.execution_task_id == Some(segment_id),
            "the parked item keeps its exact segment"
        );
        anyhow::ensure!(
            projection.paused_attempts == 1 && projection.next_attempt_at.is_none(),
            "the park adds one paused attempt and no backoff"
        );
        let attempt = sqlx::query_file!("tests/sql/task_execution/read_pause_attempt.sql", iid)
            .fetch_one(db.pool())
            .await?;
        anyhow::ensure!(
            attempt.status == "paused" && attempt.error_message.is_none(),
            "the domain attempt is sealed paused without an error"
        );

        let cursor_before_resume = barrier.cursor;
        anyhow::ensure!(
            db.request_task_resume(tid, uid, false).await?,
            "the explicit resume clears the pause"
        );
        let resumed_barrier = db.read_rebuild_barrier().await?;
        anyhow::ensure!(
            !resumed_barrier.pause_requested,
            "the resume clears the barrier pause flag"
        );
        anyhow::ensure!(
            resumed_barrier.cursor == cursor_before_resume,
            "the resume never moves the committed cursor"
        );
        if fault == RebuildCaseFault::DropResumedClaim {
            // Controlled regression: the resumed item is gone before the
            // claim, so the claim returns no matching item instead of panicking.
            sqlx::query("DELETE FROM context69.task_items WHERE id = $1")
                .bind(iid)
                .execute(db.pool())
                .await
                .context("remove the case item before the resumed claim")?;
        }
        let resumed = db
            .claim_items_fast(10)
            .await?
            .into_iter()
            .find(|item| item.id == iid)
            .context("the resumed rebuild is immediately claimable")?;
        anyhow::ensure!(
            resumed.execution_task_id == segment && resumed.execution_generation == generation,
            "the resumed claim is the same segment and generation"
        );
        anyhow::ensure!(
            resumed.attempt_count == 2,
            "the resumed claim opens the next attempt on the same segment"
        );
        anyhow::ensure!(
            mock_ref.proof_requests() >= 1,
            "the actual provider proof reached the mock"
        );
        anyhow::ensure!(
            mock_ref.data_requests() >= 2,
            "the first page reached the embedding mock in its data batches"
        );
        Ok(())
    })
    .await;

    // Cleanup always runs: release the held batch, stop and join the worker,
    // then dispose only the exact resources this case created.
    let mut cleanup_failures: Vec<String> = Vec::new();
    if let Some(mock) = mock.take() {
        mock.release_data();
        mock.shutdown().await;
    }
    if let Some(handle) = run_handle.take() {
        handle.abort();
        if let Err(error) = handle.await {
            if !error.is_cancelled() {
                cleanup_failures.push(format!("join the rebuild worker: {error}"));
            }
        }
    }

    let collection = match owned_collection {
        Some(collection) => Some(collection),
        None => db
            .read_rebuild_barrier()
            .await
            .ok()
            .and_then(|barrier| barrier.collection),
    };
    if let Some(collection) = collection.as_deref() {
        if let Err(error) = support::delete_qdrant_collection(qdrant_url, collection).await {
            cleanup_failures.push(format!("delete the owned Qdrant collection: {error:#}"));
        }
    }
    if let Err(error) = support::try_clear_runtime_settings(&db).await {
        cleanup_failures.push(format!("clear the case runtime settings: {error:#}"));
    }
    if let Some(iid) = item_id {
        if let Err(error) = support::try_delete_item_library_rows(&db, iid).await {
            cleanup_failures.push(format!("delete the item library rows: {error:#}"));
        }
    }
    if let Some(tid) = task_id {
        if let Err(error) = support::try_delete_tasks(&db, &[tid]).await {
            cleanup_failures.push(format!("delete the case task: {error:#}"));
        }
    }
    if let Some(uid) = user_id {
        if let Err(error) = support::try_delete_user(&db, uid).await {
            cleanup_failures.push(format!("delete the case user: {error:#}"));
        }
    }
    if let Some(gid) = group_id {
        if let Err(error) = support::try_delete_group(&db, gid).await {
            cleanup_failures.push(format!("delete the case group: {error:#}"));
        }
    }
    if let Err(error) = support::try_reset_rebuild_barrier(&db).await {
        cleanup_failures.push(format!("reset the case barrier: {error:#}"));
    }

    let cleanup_suffix = || {
        if cleanup_failures.is_empty() {
            String::new()
        } else {
            format!("\ncleanup also failed: {}", cleanup_failures.join("; "))
        }
    };
    match outcome {
        Ok(()) if cleanup_failures.is_empty() => Ok(()),
        Ok(()) => Err(anyhow::anyhow!(
            "cleanup failed after a passing case: {}",
            cleanup_failures.join("; ")
        )),
        Err(CaseFailure::Returned(error)) => Err(anyhow::anyhow!("{error:#}{}", cleanup_suffix())),
        Err(CaseFailure::Panicked(message)) => Err(anyhow::anyhow!(
            "the case panicked: {message}{}",
            cleanup_suffix()
        )),
    }
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL and the designated test Qdrant"]
async fn an_actual_rebuild_pauses_at_its_committed_batch_boundary_and_resumes_the_same_segment() {
    let db = connect().await;
    run_actual_rebuild_case(&db, RebuildCaseFault::None)
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL and the designated test Qdrant"]
async fn an_actual_rebuild_provider_failure_before_the_first_data_batch_terminates_and_cleans_up() {
    let db = connect().await;
    run_actual_rebuild_case(&db, RebuildCaseFault::ProviderFails)
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));

    let preflight = support::read_rebuild_fixture_preflight(&db)
        .await
        .expect("read the post-failure fixture");
    assert_eq!(
        preflight.emptiness_problem(),
        None,
        "the failed provider gate left no owned fixture behind"
    );
    let collections = support::pause_collections("http://unraid.cloud1ful.com:6336")
        .await
        .expect("list the post-failure collections");
    assert!(
        collections.is_empty(),
        "the failed provider gate left no owned collection: {collections:?}"
    );
}

#[cfg(feature = "integration-test-helpers")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SentinelObservationFault {
    None,
    Error,
    Panic,
}

/// One nonempty-fixture refusal variant.
///
/// The sentinel insert, the refusal, and the fallible observation all live in
/// one captured owned operation. The exact owned row is deleted outside that
/// boundary before any failure, panic, or assertion is reported.
#[cfg(feature = "integration-test-helpers")]
async fn run_nonempty_sentinel_variant(
    db: &Database,
    fault: SentinelObservationFault,
) -> anyhow::Result<()> {
    let sentinel = "rebuild-preflight-sentinel";
    // Clear the fixture-owned predecessor rows first; the sentinel this case
    // plants below is a foreign runtime row and is still refused.
    reset_barrier(db).await;
    let preflight = support::read_rebuild_fixture_preflight(db)
        .await
        .context("read the sentinel probe precondition")?;
    if let Some(problem) = preflight.emptiness_problem() {
        anyhow::bail!(
            "the sentinel probe requires an empty fixture, never a foreign one: {problem}"
        );
    }

    let mut sentinel_inserted = false;
    let mut refusal: Option<String> = None;
    let mut observed: Option<(i64, i64)> = None;

    let outcome = catch_case_failure(async {
        sqlx::query(
            "INSERT INTO context69.runtime_chunking_settings \
             (singleton, max_chars, overlap_chars) VALUES (TRUE, 4321, 321)",
        )
        .execute(db.pool())
        .await
        .context("plant the nonempty fixture sentinel")?;
        sentinel_inserted = true;
        refusal = Some(
            run_actual_rebuild_case(db, RebuildCaseFault::None)
                .await
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "the nonempty fixture was not refused".to_string()),
        );
        match fault {
            SentinelObservationFault::None => {
                observed = sqlx::query_as(
                    "SELECT max_chars, overlap_chars FROM context69.runtime_chunking_settings \
                     WHERE singleton = TRUE AND max_chars = 4321 AND overlap_chars = 321",
                )
                .fetch_optional(db.pool())
                .await
                .context("observe the sentinel before cleanup")?;
            }
            SentinelObservationFault::Error => {
                return Err(anyhow::anyhow!("injected sentinel observation error"));
            }
            SentinelObservationFault::Panic => {
                panic!("injected sentinel observation panic");
            }
        }
        Ok(())
    })
    .await;

    // Remove exactly this case's row before reporting any read error, panic, or
    // assertion. A foreign prior singleton is never touched.
    let deletion = if sentinel_inserted {
        sqlx::query(
            "DELETE FROM context69.runtime_chunking_settings \
             WHERE singleton = TRUE AND max_chars = 4321 AND overlap_chars = 321",
        )
        .execute(db.pool())
        .await
        .map(|result| result.rows_affected())
        .map_err(|error| {
            anyhow::Error::new(error).context("remove the sentinel fixture before reporting")
        })
    } else {
        Ok(0)
    };

    let verification: anyhow::Result<()> = async {
        match fault {
            SentinelObservationFault::None => {
                match &outcome {
                    Ok(()) => {}
                    Err(CaseFailure::Returned(error)) => {
                        anyhow::bail!(
                            "the nonempty case returned an unexpected failure: {error:#}"
                        );
                    }
                    Err(CaseFailure::Panicked(message)) => {
                        anyhow::bail!("the nonempty case panicked unexpectedly: {message}");
                    }
                }
                let refusal = refusal
                    .as_deref()
                    .context("the nonempty case recorded no refusal")?;
                anyhow::ensure!(
                    refusal.contains("not empty"),
                    "the refusal is actionable: {refusal}"
                );
                anyhow::ensure!(
                    observed == Some((4321, 321)),
                    "the refusal never mutated the nonempty fixture {sentinel}: {observed:?}"
                );
            }
            SentinelObservationFault::Error => {
                let message = match &outcome {
                    Err(CaseFailure::Returned(error)) => format!("{error:#}"),
                    _ => anyhow::bail!("expected the injected sentinel read error"),
                };
                anyhow::ensure!(
                    message.contains("injected sentinel observation error"),
                    "the original read error is preserved: {message}"
                );
            }
            SentinelObservationFault::Panic => {
                let message = match &outcome {
                    Err(CaseFailure::Panicked(message)) => message.clone(),
                    _ => anyhow::bail!("expected the injected sentinel observation panic"),
                };
                anyhow::ensure!(
                    message.contains("injected sentinel observation panic"),
                    "the original panic is preserved: {message}"
                );
            }
        }
        let removed = deletion
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error:#}"))?;
        anyhow::ensure!(
            *removed == 1,
            "the probe deleted exactly its own sentinel, removed {removed}"
        );
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.runtime_chunking_settings \
             WHERE singleton = TRUE AND max_chars = 4321 AND overlap_chars = 321",
        )
        .fetch_one(db.pool())
        .await?;
        anyhow::ensure!(
            remaining == 0,
            "the sentinel is absent after cleanup: {remaining}"
        );
        let runtime_rows: i64 = sqlx::query_scalar(
            "SELECT \
                 (SELECT count(*) FROM context69.runtime_qdrant_settings) \
               + (SELECT count(*) FROM context69.runtime_embedding_settings) \
               + (SELECT count(*) FROM context69.runtime_scheduler_settings) \
               + (SELECT count(*) FROM context69.runtime_chunking_settings) \
               + (SELECT count(*) FROM context69.runtime_file_library_settings)",
        )
        .fetch_one(db.pool())
        .await?;
        anyhow::ensure!(
            runtime_rows == 0,
            "no runtime singleton remains after cleanup: {runtime_rows}"
        );
        let barrier = db.read_rebuild_barrier().await?;
        anyhow::ensure!(
            barrier.state == RebuildBarrierState::Idle,
            "the refusal never touched the barrier"
        );
        Ok(())
    }
    .await;

    match (deletion, verification) {
        (Ok(_), Ok(())) => Ok(()),
        (Err(cleanup), Ok(())) => Err(anyhow::anyhow!(
            "cleanup failed after a passing sentinel variant: {cleanup:#}"
        )),
        (Ok(_), Err(error)) => Err(error),
        (Err(cleanup), Err(error)) => Err(anyhow::anyhow!(
            "{error:#}\ncleanup also failed: {cleanup:#}"
        )),
    }
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL and the designated test Qdrant"]
async fn an_actual_rebuild_refuses_a_nonempty_runtime_fixture_before_any_write() {
    let db = connect().await;
    for fault in [
        SentinelObservationFault::None,
        SentinelObservationFault::Error,
        SentinelObservationFault::Panic,
    ] {
        run_nonempty_sentinel_variant(&db, fault)
            .await
            .unwrap_or_else(|error| panic!("the sentinel variant {fault:?} failed: {error:#}"));
    }
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL and the designated test Qdrant"]
async fn an_actual_rebuild_missing_resumed_claim_reports_and_cleans_up() {
    let db = connect().await;
    let error = run_actual_rebuild_case(&db, RebuildCaseFault::DropResumedClaim)
        .await
        .expect_err("the missing resumed claim is reported, not swallowed");
    assert!(
        format!("{error:#}").contains("the resumed rebuild is immediately claimable"),
        "the original claim diagnostic is preserved: {error:#}"
    );
    let preflight = support::read_rebuild_fixture_preflight(&db)
        .await
        .expect("read the post-claim-failure fixture");
    assert_eq!(
        preflight.emptiness_problem(),
        None,
        "the missing resumed claim left no owned fixture behind"
    );
    let collections = support::pause_collections("http://unraid.cloud1ful.com:6336")
        .await
        .expect("list the post-claim-failure collections");
    assert!(
        collections.is_empty(),
        "the missing resumed claim left no owned collection: {collections:?}"
    );
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_document_rebuild_stages_fresh_chunks_verifies_hashes_and_swaps_by_stored_id() {
    let db = connect().await;
    reset_barrier(&db).await;

    let group_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("staging-{}", Uuid::new_v4()),
        "staging fixture",
        format!("test/staging-{}", Uuid::new_v4()),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the staging group");

    let (document_id, _) = support::seed_reindex_document(&db, group_id, 5)
        .await
        .expect("seed the canonical document");
    // A canonical body that produces no chunks must still stage a progress row.
    let (empty_document_id, empty_chunks) = support::seed_reindex_document(&db, group_id, 0)
        .await
        .expect("seed the empty canonical document");
    assert_eq!(empty_chunks, 0, "an empty body produces no chunks");

    let row = db
        .list_rebuild_documents_page(Some(document_id - 1), 10)
        .await
        .expect("read the rebuild document page")
        .into_iter()
        .find(|row| row.document_id == document_id)
        .expect("the seeded document is pageable")
        .clone();
    let empty_row = db
        .list_rebuild_documents_page(Some(empty_document_id - 1), 10)
        .await
        .expect("read the empty rebuild document page")
        .into_iter()
        .find(|row| row.document_id == empty_document_id)
        .expect("the empty document is pageable")
        .clone();
    assert_eq!(
        row.body_text.trim().is_empty(),
        false,
        "the canonical body is stored"
    );

    // One current translation survives the rebuild with its exact ids.
    let translation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.document_translation_versions \
         (id, document_id, target_locale, source_locale, source_record_hash, provider_key, \
          provider_config_hash, translated_title, translated_summary, translated_body_text) \
         VALUES ($1, $2, 'zh-CN', 'en', $3, 'deepl', 'config-hash', 'translated', NULL, 'body')",
    )
    .bind(translation_id)
    .bind(document_id)
    .bind(&row.record_hash)
    .execute(db.pool())
    .await
    .expect("seed the current translation version");
    let mut translation_chunk_ids = Vec::new();
    for index in 0..2i32 {
        let chunk_id = Uuid::new_v4();
        translation_chunk_ids.push(chunk_id);
        sqlx::query(
            "INSERT INTO context69.document_translation_chunks \
             (id, translation_id, document_id, target_locale, chunk_index, chunk_text) \
             VALUES ($1, $2, $3, 'zh-CN', $4, $5)",
        )
        .bind(chunk_id)
        .bind(translation_id)
        .bind(document_id)
        .bind(index)
        .bind(format!("translated chunk {index}"))
        .execute(db.pool())
        .await
        .expect("seed a current translation chunk");
    }

    let (owner_item, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the barrier claim");
    db.begin_rebuild_in(&mut tx, owner_item, owner_token, FINGERPRINT, COLLECTION)
        .await
        .expect("claim the rebuild barrier");
    tx.commit().await.expect("commit the barrier claim");

    let budget = context69::chunking::ChunkingRuntime::new(
        context69::chunking::ChunkBudget::chars(context69::chunking::ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        }),
    )
    .resolve_snapshot()
    .expect("resolve the staging budget");
    let normalized = context69::domain::NormalizedDocument {
        external_id: row.external_id.clone(),
        title: row.title.clone(),
        summary: row.summary.clone(),
        body_text: row.body_text.clone(),
        source_uri: row.source_uri.clone(),
        published_at: row.published_at,
        updated_at: row.updated_at_source,
        metadata_json: row.metadata_json.clone(),
        record_hash: row.record_hash.clone(),
    };
    let chunks = context69::chunking::chunk_document_resolved(
        document_id,
        "reindex-fixture",
        &normalized,
        &budget,
    )
    .expect("chunk the canonical body");
    assert!(chunks.len() >= 5, "the fixture body produces chunks");

    let staged = db
        .stage_rebuild_document(
            COLLECTION,
            &row,
            &chunks,
            owner_item,
            owner_token,
            FINGERPRINT,
        )
        .await
        .expect("stage the document chunks");
    assert!(staged, "the owner still holds the barrier");
    let staged_empty = db
        .stage_rebuild_document(
            COLLECTION,
            &empty_row,
            &[],
            owner_item,
            owner_token,
            FINGERPRINT,
        )
        .await
        .expect("stage the empty document");
    assert!(staged_empty, "the empty document stages a progress row");

    let counts = db
        .count_rebuild_staging_chunks(COLLECTION)
        .await
        .expect("count the staged chunks");
    assert_eq!(counts.original, i64::try_from(chunks.len()).unwrap());
    assert_eq!(counts.translated, 2);
    let verification = db
        .verify_rebuild_staging(COLLECTION)
        .await
        .expect("verify the staged snapshot");
    assert_eq!(verification.document_hash_mismatch, 0);
    assert_eq!(verification.chunk_count_mismatch, 0);
    assert_eq!(verification.chunk_hash_mismatch, 0);
    assert_eq!(verification.staged_documents, 2);

    let previous_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM context69.document_chunks WHERE document_id = $1")
            .bind(document_id)
            .fetch_all(db.pool())
            .await
            .expect("read the legacy chunk ids");
    let mut tx = db.pool().begin().await.expect("begin the activation swap");
    db.swap_rebuild_chunks_in(&mut tx, COLLECTION)
        .await
        .expect("swap the staged chunks");
    tx.commit().await.expect("commit the activation swap");

    let live_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM context69.document_chunks WHERE document_id = $1 ORDER BY chunk_index",
    )
    .bind(document_id)
    .fetch_all(db.pool())
    .await
    .expect("read the live chunk ids");
    let expected: Vec<Uuid> = chunks.iter().map(|chunk| chunk.id).collect();
    assert_eq!(live_ids, expected, "live ids equal the staged target ids");
    assert!(
        previous_ids.iter().all(|id| !live_ids.contains(id)),
        "the legacy ids were replaced"
    );
    let empty_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.document_chunks WHERE document_id = $1")
            .bind(empty_document_id)
            .fetch_one(db.pool())
            .await
            .expect("count the empty document's chunks");
    assert_eq!(empty_count, 0);
    let live_translation_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM context69.document_translation_chunks \
         WHERE translation_id = $1 ORDER BY chunk_index",
    )
    .bind(translation_id)
    .fetch_all(db.pool())
    .await
    .expect("read the current translation chunk ids");
    assert_eq!(
        live_translation_ids, translation_chunk_ids,
        "current translation vectors survive activation with their ids"
    );

    let mut connection = db.pool().acquire().await.expect("acquire cleanup");
    db.delete_rebuild_staging_in(&mut connection, COLLECTION)
        .await
        .expect("delete the staged rows");
    drop(connection);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.document_rebuild_staging_documents")
            .fetch_one(db.pool())
            .await
            .expect("count remaining staging rows");
    assert_eq!(remaining, 0);

    sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("delete the staging documents");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("delete the staging group");
    reset_barrier(&db).await;
}

#[cfg(feature = "integration-test-helpers")]
async fn seed_barrier_fixture_group(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("barrier-acceptance-{}", Uuid::new_v4()),
        "barrier acceptance fixture",
        format!("test/barrier-acceptance-{}", Uuid::new_v4()),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the barrier acceptance group")
}

#[cfg(feature = "integration-test-helpers")]
fn rebuild_row_normalized(
    row: &context69::db::RebuildDocumentRow,
) -> context69::domain::NormalizedDocument {
    context69::domain::NormalizedDocument {
        external_id: row.external_id.clone(),
        title: row.title.clone(),
        summary: row.summary.clone(),
        body_text: row.body_text.clone(),
        source_uri: row.source_uri.clone(),
        published_at: row.published_at,
        updated_at: row.updated_at_source,
        metadata_json: row.metadata_json.clone(),
        record_hash: row.record_hash.clone(),
    }
}

fn pinned_budget() -> context69::chunking::ChunkBudget {
    context69::chunking::ChunkBudget::chars(context69::chunking::ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    })
}

fn character_budget(
    max_chars: usize,
    overlap_chars: usize,
) -> context69::chunking::ResolvedChunkBudget {
    context69::chunking::ChunkingRuntime::new(context69::chunking::ChunkBudget::chars(
        context69::chunking::ChunkingConfig {
            max_chars,
            overlap_chars,
        },
    ))
    .resolve_snapshot()
    .expect("resolve the fixture chunk budget")
}

#[cfg(feature = "integration-test-helpers")]
fn chunk_rebuild_document(
    document_id: i64,
    row: &context69::db::RebuildDocumentRow,
    budget: &context69::chunking::ResolvedChunkBudget,
    generation: Option<i64>,
) -> Vec<context69::domain::DocumentChunk> {
    context69::chunking::chunk_document_iter_resolved(
        document_id,
        &row.source_key,
        &rebuild_row_normalized(row),
        budget,
        generation,
    )
    .expect("chunk the canonical rebuild body")
    .collect::<Result<Vec<_>, _>>()
    .expect("the canonical body chunks without error")
}

#[cfg(feature = "integration-test-helpers")]
async fn live_chunk_ids(db: &Database, document_id: i64) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM context69.document_chunks WHERE document_id = $1 ORDER BY chunk_index",
    )
    .bind(document_id)
    .fetch_all(db.pool())
    .await
    .expect("read the live chunk ids")
}

#[cfg(feature = "integration-test-helpers")]
async fn reset_acceptance_fixture(db: &Database, group_id: i64, collections: &[&str]) {
    let collections: Vec<String> = collections
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    for statement in [
        "DELETE FROM context69.document_chunk_rebuild_staging WHERE target_collection = ANY($1)",
        "DELETE FROM context69.document_translation_rebuild_staging WHERE target_collection = ANY($1)",
        "DELETE FROM context69.document_rebuild_staging_documents WHERE target_collection = ANY($1)",
    ] {
        sqlx::query(statement)
            .bind(&collections)
            .execute(db.pool())
            .await
            .expect("delete the fixture staging rows");
    }
    for statement in [
        "DELETE FROM context69.document_translation_chunks WHERE document_id IN (SELECT id FROM context69.documents WHERE group_id = $1)",
        "DELETE FROM context69.document_translation_versions WHERE document_id IN (SELECT id FROM context69.documents WHERE group_id = $1)",
        "DELETE FROM context69.document_chunks WHERE document_id IN (SELECT id FROM context69.documents WHERE group_id = $1)",
        "DELETE FROM context69.document_versions WHERE document_id IN (SELECT id FROM context69.documents WHERE group_id = $1)",
        "DELETE FROM context69.documents WHERE group_id = $1",
    ] {
        sqlx::query(statement)
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("delete the fixture document rows");
    }
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("delete the fixture group");
    sqlx::query("DELETE FROM context69.vector_index_state WHERE collection_name = ANY($1)")
        .bind(&collections)
        .execute(db.pool())
        .await
        .expect("delete the fixture vector index rows");
    clear_runtime_settings(db).await;
    reset_barrier(db).await;
}

/// The newest saved chunk budget governs the fresh chunk boundaries, and the
/// activation installs exactly the target ids generated under its pinned seed.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_rebuild_uses_the_latest_saved_budget_and_activates_its_target_ids() {
    const TARGET: &str = "barrier-latest-budget__target";
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
    sqlx::query(
        "UPDATE context69.runtime_chunking_settings \
         SET max_chars = 500, overlap_chars = 0 WHERE singleton = TRUE",
    )
    .execute(db.pool())
    .await
    .expect("save the newest chunk budget");

    let group_id = seed_barrier_fixture_group(&db).await;
    let (document_id, _) = support::seed_reindex_document(&db, group_id, 2)
        .await
        .expect("seed the canonical document");
    let row = db
        .list_rebuild_documents_page(Some(document_id - 1), 10)
        .await
        .expect("read the rebuild document page")
        .into_iter()
        .find(|row| row.document_id == document_id)
        .expect("the seeded document is pageable")
        .clone();

    let old_chunks = chunk_rebuild_document(document_id, &row, &character_budget(1000, 100), None);
    let (max_chars, overlap_chars): (i64, i64) = sqlx::query_as(
        "SELECT max_chars, overlap_chars FROM context69.runtime_chunking_settings \
         WHERE singleton = TRUE",
    )
    .fetch_one(db.pool())
    .await
    .expect("read the newest saved budget");
    assert_eq!((max_chars, overlap_chars), (500, 0));
    let saved_budget = character_budget(
        usize::try_from(max_chars).unwrap(),
        usize::try_from(overlap_chars).unwrap(),
    );

    let mut connection = db.pool().acquire().await.expect("acquire a connection");
    let generation = db
        .next_ingest_generation_in(&mut connection)
        .await
        .expect("reserve the fixture generation");
    drop(connection);
    let target_chunks = chunk_rebuild_document(document_id, &row, &saved_budget, Some(generation));
    assert!(
        target_chunks.len() > old_chunks.len(),
        "the newer boundary set produces more chunks: {} vs {}",
        target_chunks.len(),
        old_chunks.len()
    );
    assert!(
        target_chunks
            .iter()
            .all(|chunk| chunk.text.chars().count() <= 500),
        "every fresh boundary stays within the newest saved budget"
    );
    assert!(
        target_chunks
            .iter()
            .all(|chunk| old_chunks.iter().all(|old| old.id != chunk.id)),
        "the pinned generation yields different target ids than the old build"
    );

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the rebuild claim");
    let (outcome, resumed) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            Some(generation),
        )
        .await
        .expect("claim the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    assert!(!resumed, "a fresh claim never resumes a target");
    tx.commit().await.expect("commit the rebuild claim");

    assert!(
        db.stage_rebuild_document(
            TARGET,
            &row,
            &target_chunks,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
        )
        .await
        .expect("stage the target chunks"),
        "the owner still holds the barrier"
    );
    let staged = db
        .list_rebuild_staging_chunks_page(TARGET, None, None, 1000)
        .await
        .expect("read the staged target chunks");
    let expected_ids: Vec<Uuid> = target_chunks.iter().map(|chunk| chunk.id).collect();
    let staged_ids: Vec<Uuid> = staged.iter().map(|chunk| chunk.chunk_id).collect();
    assert_eq!(
        staged_ids, expected_ids,
        "the target ids are staged exactly"
    );
    let staged_texts: Vec<&str> = staged
        .iter()
        .map(|chunk| chunk.chunk_text.as_str())
        .collect();
    let target_texts: Vec<&str> = target_chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect();
    assert_eq!(
        staged_texts, target_texts,
        "the staged boundaries equal the newest saved budget"
    );

    let counts = db
        .count_rebuild_staging_chunks(TARGET)
        .await
        .expect("count the staged snapshot");
    let total = counts.original + counts.translated;
    let mut connection = db.pool().acquire().await.expect("acquire completion");
    assert!(
        db.complete_rebuild_staging_in(
            &mut connection,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            total,
        )
        .await
        .expect("complete the staged snapshot")
    );
    drop(connection);
    let mut tx = db.pool().begin().await.expect("begin the activation");
    let activated = db
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
                rebuilt_chunks: target_chunks.len(),
            },
        )
        .await
        .expect("activate the staged target");
    assert!(activated.is_some(), "the owner activates its target");
    tx.commit().await.expect("commit the activation");

    assert_eq!(
        live_chunk_ids(&db, document_id).await,
        expected_ids,
        "activation installs exactly the target ids"
    );
    let active = db
        .read_active_vector_index_state()
        .await
        .expect("read the active vector index")
        .expect("the target is active");
    assert_eq!(active.collection_name, TARGET);

    reset_acceptance_fixture(&db, group_id, &[COLLECTION, TARGET]).await;
}

/// A barrier created before the durable document snapshot existed is never
/// resumed or activated, and the old active state keeps serving.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_legacy_barrier_without_a_pinned_snapshot_is_refused_and_keeps_the_old_active_state() {
    const TARGET: &str = "barrier-legacy__target";
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    clear_runtime_settings(&db).await;
    let desired = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;
    let group_id = seed_barrier_fixture_group(&db).await;
    let (document_id, _) = support::seed_reindex_document(&db, group_id, 1)
        .await
        .expect("seed the canonical document");
    let old_ids = live_chunk_ids(&db, document_id).await;
    let active_before = db
        .read_active_vector_index_state()
        .await
        .expect("read the old active index")
        .expect("the old active index exists");

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the legacy claim");
    db.begin_rebuild_in(
        &mut tx,
        owner,
        owner_token,
        &desired.embedding_fingerprint,
        TARGET,
    )
    .await
    .expect("start the legacy rebuild");
    tx.commit().await.expect("commit the legacy claim");
    let legacy = db
        .read_rebuild_staging_state()
        .await
        .expect("read the legacy staging state");
    assert!(legacy.chunk_budget.is_none());
    assert!(legacy.generation.is_none());

    let state = context69::db::VectorIndexState {
        collection_name: TARGET,
        fingerprint: &desired.embedding_fingerprint,
        embedding_base_url: "https://embeddings.example/v1",
        embedding_model: "model-a",
        dimensions: 8,
        rebuilt_chunks: 0,
    };
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the legacy completion");
    let error = db
        .complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
        .await
        .expect_err("a legacy barrier never activates");
    assert!(
        error
            .to_string()
            .contains("records no pinned document snapshot"),
        "the refusal is explicit: {error}"
    );
    tx.rollback()
        .await
        .expect("roll back the refused completion");

    // An expired owner may not take over a legacy barrier either.
    set_item_lease(&db, owner, Uuid::new_v4(), false).await;
    let (next_owner, next_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the legacy takeover");
    let (outcome, _) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            next_owner,
            next_token,
            &desired.embedding_fingerprint,
            TARGET,
            None,
        )
        .await
        .expect("check the legacy takeover");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::LegacyStaging)
    );
    tx.rollback().await.expect("roll back the refused takeover");

    // The same refusal applies after the legacy barrier is recorded failed.
    let mut tx = db.pool().begin().await.expect("begin the legacy failure");
    assert!(
        db.fail_rebuild_in(&mut tx, owner, owner_token, "legacy rebuild failed")
            .await
            .expect("record the legacy failure")
    );
    tx.commit().await.expect("commit the legacy failure");
    let mut tx = db.pool().begin().await.expect("begin the failed takeover");
    let (outcome, _) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            next_owner,
            next_token,
            &desired.embedding_fingerprint,
            TARGET,
            None,
        )
        .await
        .expect("check the failed takeover");
    assert_eq!(
        outcome,
        RebuildStartOutcome::Refused(RebuildRefusalReason::LegacyStaging)
    );
    tx.rollback().await.expect("roll back the failed takeover");

    let active_after = db
        .read_active_vector_index_state()
        .await
        .expect("read the active index after the refusals")
        .expect("the old active index still exists");
    assert_eq!(active_after.collection_name, active_before.collection_name);
    assert_eq!(live_chunk_ids(&db, document_id).await, old_ids);

    reset_acceptance_fixture(&db, group_id, &[COLLECTION, TARGET]).await;
}

/// A failed task may still own a running legacy barrier; the explicit discard
/// clears it and a fresh checked claim then pins a real snapshot.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failed_task_with_a_running_legacy_barrier_can_discard_and_start_fresh() {
    const TARGET: &str = "barrier-legacy-cancel__target";
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    clear_runtime_settings(&db).await;
    let desired = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;
    let failed = seed_task_item_full(&db, "failed", "vector_rebuild").await;
    let mut tx = db.pool().begin().await.expect("begin the legacy claim");
    db.begin_rebuild_in(
        &mut tx,
        failed.item_id,
        failed.token,
        &desired.embedding_fingerprint,
        TARGET,
    )
    .await
    .expect("start the legacy rebuild");
    tx.commit().await.expect("commit the legacy claim");
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Running);

    assert!(
        db.discard_failed_rebuild(failed.task_id)
            .await
            .expect("discard the running legacy barrier")
    );
    assert_eq!(barrier_state(&db).await, RebuildBarrierState::Idle);

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the fresh claim");
    let (outcome, _) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            None,
        )
        .await
        .expect("claim the fresh rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the fresh claim");
    let staged = db
        .read_rebuild_staging_state()
        .await
        .expect("read the fresh pinned snapshot");
    assert!(staged.chunk_budget.is_some());
    assert!(staged.generation.is_some());

    reset_barrier(&db).await;
    clear_runtime_settings(&db).await;
}

/// A claim blocked on the epoch still pins the chunk settings committed while
/// it waited, and a later settings save never rewrites a resumed snapshot.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_fresh_claim_pins_the_committed_saved_budget_under_the_epoch_lock() {
    const TARGET: &str = "barrier-committed-budget__target";
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    clear_runtime_settings(&db).await;
    let desired = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;

    let mut gate = db.pool().begin().await.expect("begin the settings gate");
    db.lock_search_registry_epoch_in(&mut gate)
        .await
        .expect("hold the epoch");
    sqlx::query(
        "UPDATE context69.runtime_chunking_settings \
         SET max_chars = 500, overlap_chars = 0 WHERE singleton = TRUE",
    )
    .execute(&mut *gate)
    .await
    .expect("commit the new budget under the epoch");
    let fingerprint = desired.embedding_fingerprint.clone();
    let claim_db = db.clone();
    let claim = tokio::spawn(async move {
        let (owner, owner_token) = seed_task_item(&claim_db, "running").await;
        let mut tx = claim_db
            .pool()
            .begin()
            .await
            .expect("begin the blocked claim");
        let result = claim_db
            .begin_rebuild_checked_with_budget_in(
                &mut tx,
                owner,
                owner_token,
                &fingerprint,
                TARGET,
                None,
            )
            .await;
        match result {
            Ok((outcome, resumed)) => {
                tx.commit().await.expect("commit the blocked claim");
                Ok((outcome, resumed))
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(!claim.is_finished(), "the claim waits for the epoch lock");
    gate.commit().await.expect("commit the saved budget");

    let (outcome, resumed) = claim
        .await
        .expect("join the claim")
        .expect("claim the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    assert!(!resumed);
    let staged = db
        .read_rebuild_staging_state()
        .await
        .expect("read the committed pinned budget");
    let pinned = context69::chunking::parse_chunk_budget_value(
        staged.chunk_budget.as_ref().expect("the budget is pinned"),
    )
    .expect("parse the pinned budget");
    assert_eq!(pinned.chars.max_chars, 500);
    assert_eq!(pinned.chars.overlap_chars, 0);
    assert!(staged.generation.is_some());

    reset_barrier(&db).await;
    clear_runtime_settings(&db).await;
}

/// A settings save while the rebuild is paused never replaces the pinned
/// budget or generation, and the resumed page skips the completed document.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_paused_resume_keeps_the_pinned_budget_generation_and_completed_documents() {
    const TARGET: &str = "barrier-pinned-budget__target";
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
    sqlx::query(
        "UPDATE context69.runtime_chunking_settings \
         SET max_chars = 1000, overlap_chars = 100 WHERE singleton = TRUE",
    )
    .execute(db.pool())
    .await
    .expect("save the pinned chunk budget");

    let group_id = seed_barrier_fixture_group(&db).await;
    let (first_id, _) = support::seed_reindex_document(&db, group_id, 1)
        .await
        .expect("seed the first document");
    let (second_id, _) = support::seed_reindex_document(&db, group_id, 1)
        .await
        .expect("seed the second document");
    let first_row = db
        .list_rebuild_documents_page(Some(first_id - 1), 10)
        .await
        .expect("read the first page")
        .into_iter()
        .find(|row| row.document_id == first_id)
        .expect("the first document is pageable")
        .clone();
    let second_row = db
        .list_rebuild_documents_page(Some(first_id), 10)
        .await
        .expect("read the second page")
        .into_iter()
        .find(|row| row.document_id == second_id)
        .expect("the second document is pageable")
        .clone();

    let pinned_value = context69::chunking::chunk_budget_to_value(
        &db.get_runtime_settings()
            .await
            .expect("read the pinned runtime settings")
            .expect("the pinned runtime settings exist")
            .chunking
            .chunk_budget(),
    );
    let generation = 7_071_i64;
    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the fresh claim");
    let (outcome, resumed) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            Some(generation),
        )
        .await
        .expect("claim the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    assert!(!resumed, "a fresh claim never resumes a target");
    tx.commit().await.expect("commit the fresh claim");

    let first_chunks = chunk_rebuild_document(
        first_id,
        &first_row,
        &character_budget(1000, 100),
        Some(generation),
    );
    assert!(
        db.stage_rebuild_document(
            TARGET,
            &first_row,
            &first_chunks,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
        )
        .await
        .expect("stage the first document")
    );
    let staged_state = db
        .read_rebuild_staging_state()
        .await
        .expect("read the pinned staging state");
    assert_eq!(staged_state.chunk_budget, Some(pinned_value.clone()));
    assert_eq!(staged_state.generation, Some(generation));
    assert_eq!(staged_state.staged_document_id, Some(first_id));

    let mut tx = db.pool().begin().await.expect("begin the pause");
    assert!(
        db.request_rebuild_pause_in(&mut tx, owner, owner_token)
            .await
            .expect("request the pause")
    );
    tx.commit().await.expect("commit the pause");

    sqlx::query(
        "UPDATE context69.runtime_chunking_settings \
         SET max_chars = 250, overlap_chars = 0 WHERE singleton = TRUE",
    )
    .execute(db.pool())
    .await
    .expect("save a different budget while paused");
    // The parked item is no longer a live writer, so its explicit resume is
    // the only path that may reopen the same owned barrier.
    sqlx::query("UPDATE context69.task_items SET status = 'queued' WHERE id = $1")
        .bind(owner)
        .execute(db.pool())
        .await
        .expect("park the rebuild item");

    let mut tx = db.pool().begin().await.expect("begin the resume");
    assert!(
        db.resume_rebuild_in(&mut tx, owner)
            .await
            .expect("resume the paused rebuild")
    );
    let (outcome, resumed) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            None,
        )
        .await
        .expect("resume the rebuild claim");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    assert!(resumed, "the same owner resumes the recorded target");
    tx.commit().await.expect("commit the resumed claim");

    let resumed_state = db
        .read_rebuild_staging_state()
        .await
        .expect("read the resumed staging state");
    assert_eq!(
        resumed_state.chunk_budget,
        Some(pinned_value.clone()),
        "a settings save never replaces the pinned budget"
    );
    assert_eq!(
        resumed_state.generation,
        Some(generation),
        "a settings save never replaces the pinned generation"
    );
    assert_eq!(resumed_state.staged_document_id, Some(first_id));

    let second_chunks = chunk_rebuild_document(
        second_id,
        &second_row,
        &character_budget(1000, 100),
        Some(generation),
    );
    assert!(
        db.stage_rebuild_document(
            TARGET,
            &second_row,
            &second_chunks,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
        )
        .await
        .expect("stage the second document")
    );
    let staged_after = db
        .list_rebuild_staging_chunks_page(TARGET, None, None, 1000)
        .await
        .expect("read the staged chunks");
    let first_ids: Vec<Uuid> = staged_after
        .iter()
        .filter(|chunk| chunk.document_id == first_id)
        .map(|chunk| chunk.chunk_id)
        .collect();
    let expected_first: Vec<Uuid> = first_chunks.iter().map(|chunk| chunk.id).collect();
    assert_eq!(
        first_ids, expected_first,
        "resuming never re-chunks a completed document"
    );
    assert_eq!(
        db.read_rebuild_staging_state()
            .await
            .expect("read the advanced staging state")
            .staged_document_id,
        Some(second_id),
        "the resumed page advances only past the new document"
    );

    reset_acceptance_fixture(&db, group_id, &[COLLECTION, TARGET]).await;
}

/// A current document without its canonical body blocks the staging pass
/// before any staged row is written.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_rebuild_without_a_canonical_body_fails_before_writing_any_staging() {
    const TARGET: &str = "barrier-missing-body__target";
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
    let group_id = seed_barrier_fixture_group(&db).await;
    let document_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.documents \
         (group_id, visibility, source_key, external_id, title, source_uri, updated_at_source, \
          metadata_json, record_hash) \
         VALUES ($1, 'public', 'reindex-fixture', $2, 'missing body', \
                 'https://example.com/rebuild', now(), '{}'::jsonb, $3) \
         RETURNING id",
    )
    .bind(group_id)
    .bind(format!("missing-{}", Uuid::new_v4()))
    .bind(format!("missing-hash-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed the document without a canonical body");

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the rebuild claim");
    let outcome = db
        .begin_rebuild_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
        )
        .await
        .expect("claim the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the rebuild claim");

    let missing = db
        .list_rebuild_missing_documents_page(None, 64)
        .await
        .expect("read the documents without a canonical body");
    assert!(
        missing.contains(&document_id),
        "the worker's fail-closed precondition reports the document: {missing:?}"
    );
    let counts = db
        .count_rebuild_staging_chunks(TARGET)
        .await
        .expect("count the staged snapshot");
    assert_eq!((counts.original, counts.translated), (0, 0));
    let staged_documents: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.document_rebuild_staging_documents \
         WHERE target_collection = $1",
    )
    .bind(TARGET)
    .fetch_one(db.pool())
    .await
    .expect("count the staged documents");
    assert_eq!(staged_documents, 0, "no staging row is written");

    reset_acceptance_fixture(&db, group_id, &[COLLECTION, TARGET]).await;
}

/// A hash or chunk-count mismatch refuses the activation transaction and
/// leaves the previous chunks and active pointer exactly as they were.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_hash_count_or_membership_mismatch_refuses_activation_and_keeps_the_old_state() {
    const TARGET: &str = "barrier-mismatch__target";
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
    let group_id = seed_barrier_fixture_group(&db).await;
    let (document_id, _) = support::seed_reindex_document(&db, group_id, 1)
        .await
        .expect("seed the canonical document");
    let row = db
        .list_rebuild_documents_page(Some(document_id - 1), 10)
        .await
        .expect("read the rebuild document page")
        .into_iter()
        .find(|row| row.document_id == document_id)
        .expect("the seeded document is pageable")
        .clone();

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the rebuild claim");
    let (outcome, _) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            Some(11),
        )
        .await
        .expect("claim the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the rebuild claim");

    let chunks = chunk_rebuild_document(document_id, &row, &character_budget(1000, 100), Some(11));
    assert!(
        db.stage_rebuild_document(
            TARGET,
            &row,
            &chunks,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
        )
        .await
        .expect("stage the target chunks")
    );
    let counts = db
        .count_rebuild_staging_chunks(TARGET)
        .await
        .expect("count the staged snapshot");
    let total = counts.original + counts.translated;
    let mut connection = db.pool().acquire().await.expect("acquire completion");
    assert!(
        db.complete_rebuild_staging_in(
            &mut connection,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            total,
        )
        .await
        .expect("complete the staged snapshot")
    );
    drop(connection);

    let old_ids = live_chunk_ids(&db, document_id).await;
    let state = context69::db::VectorIndexState {
        collection_name: TARGET,
        fingerprint: &desired.embedding_fingerprint,
        embedding_base_url: "https://embeddings.example/v1",
        embedding_model: "model-a",
        dimensions: 8,
        rebuilt_chunks: chunks.len(),
    };

    // The canonical document moved under the staged snapshot.
    sqlx::query("UPDATE context69.documents SET record_hash = $1 WHERE id = $2")
        .bind("moved-hash")
        .bind(document_id)
        .execute(db.pool())
        .await
        .expect("move the canonical document hash");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the refused activation");
    let error = db
        .complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
        .await
        .expect_err("a document hash mismatch refuses activation");
    assert!(
        error.to_string().contains("document hash mismatch"),
        "the refusal names the mismatch: {error}"
    );
    tx.rollback()
        .await
        .expect("roll back the refused activation");
    assert_eq!(
        db.read_active_vector_index_state()
            .await
            .expect("read the active vector index")
            .expect("the previous collection is still active")
            .collection_name,
        COLLECTION
    );
    assert_eq!(
        live_chunk_ids(&db, document_id).await,
        old_ids,
        "the refused activation keeps the old chunks"
    );
    let staged_documents: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.document_rebuild_staging_documents \
         WHERE target_collection = $1",
    )
    .bind(TARGET)
    .fetch_one(db.pool())
    .await
    .expect("count the retained staging rows");
    assert_eq!(
        staged_documents, 1,
        "the refused activation keeps the staged snapshot for a retry"
    );

    // Restore the hash and corrupt the staged chunk count instead.
    sqlx::query("UPDATE context69.documents SET record_hash = $1 WHERE id = $2")
        .bind(&row.record_hash)
        .bind(document_id)
        .execute(db.pool())
        .await
        .expect("restore the canonical document hash");
    sqlx::query(
        "UPDATE context69.document_rebuild_staging_documents \
         SET chunk_count = chunk_count + 1 WHERE target_collection = $1 AND document_id = $2",
    )
    .bind(TARGET)
    .bind(document_id)
    .execute(db.pool())
    .await
    .expect("corrupt the staged chunk count");
    let mut tx = db.pool().begin().await.expect("begin the second refusal");
    let error = db
        .complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
        .await
        .expect_err("a staged chunk count mismatch refuses activation");
    assert!(
        error.to_string().contains("chunk count mismatch"),
        "the refusal names the mismatch: {error}"
    );
    tx.rollback().await.expect("roll back the second refusal");
    assert_eq!(
        db.read_active_vector_index_state()
            .await
            .expect("read the active vector index")
            .expect("the previous collection is still active")
            .collection_name,
        COLLECTION
    );

    sqlx::query(
        "UPDATE context69.document_rebuild_staging_documents \
         SET chunk_count = $1 WHERE target_collection = $2 AND document_id = $3",
    )
    .bind(i32::try_from(chunks.len()).unwrap())
    .bind(TARGET)
    .bind(document_id)
    .execute(db.pool())
    .await
    .expect("restore the staged chunk count");

    // A canonical document added after the staged snapshot is not part of the
    // target and must refuse activation instead of being silently omitted.
    let (unstaged_document_id, _) = support::seed_reindex_document(&db, group_id, 1)
        .await
        .expect("seed a document outside the staged snapshot");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the membership refusal");
    let error = db
        .complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
        .await
        .expect_err("a document outside the staged snapshot refuses activation");
    assert!(
        error.to_string().contains("document membership mismatch"),
        "the refusal names the membership mismatch: {error}"
    );
    tx.rollback()
        .await
        .expect("roll back the membership refusal");
    assert_eq!(
        db.read_active_vector_index_state()
            .await
            .expect("read the active vector index")
            .expect("the previous collection is still active")
            .collection_name,
        COLLECTION
    );
    sqlx::query("DELETE FROM context69.document_chunks WHERE document_id = $1")
        .bind(unstaged_document_id)
        .execute(db.pool())
        .await
        .expect("remove the unstaged document chunks");
    sqlx::query("DELETE FROM context69.document_versions WHERE document_id = $1")
        .bind(unstaged_document_id)
        .execute(db.pool())
        .await
        .expect("remove the unstaged document versions");
    sqlx::query("DELETE FROM context69.documents WHERE id = $1")
        .bind(unstaged_document_id)
        .execute(db.pool())
        .await
        .expect("remove the unstaged document");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the accepted activation");
    assert!(
        db.complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
            .await
            .expect("the restored snapshot activates")
            .is_some()
    );
    tx.commit().await.expect("commit the accepted activation");
    assert_eq!(
        live_chunk_ids(&db, document_id).await,
        chunks.iter().map(|chunk| chunk.id).collect::<Vec<_>>()
    );

    reset_acceptance_fixture(&db, group_id, &[COLLECTION, TARGET]).await;
}

/// The quota projection discounts exactly the producer-proven originals of a
/// replacement entry once its collection is no longer active, keeps the
/// translation ids and the history, and refuses an activation that still
/// exceeds the policy.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_quota_projection_discounts_proven_replaced_originals_and_refuses_overflow() {
    const TARGET: &str = "barrier-quota__target";
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
    let group_id = seed_barrier_fixture_group(&db).await;
    let (document_id, _) = support::seed_reindex_document(&db, group_id, 4)
        .await
        .expect("seed the canonical document with four old originals");
    let file_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, external_id, filename, media_type, size_bytes, sha256, \
          storage_rel_path, ingest_status, visibility) \
         VALUES ($1, $2, $3, 'quota.txt', 'text/plain', 1, $4, $5, 'succeeded', 'public')",
    )
    .bind(file_id)
    .bind(group_id)
    .bind(format!("quota-{file_id}"))
    .bind("3".repeat(64))
    .bind(format!("objects/{file_id}"))
    .execute(db.pool())
    .await
    .expect("seed the owning library file");
    sqlx::query(
        "INSERT INTO context69.library_file_documents \
         (file_id, document_id, group_id, visibility, section_key, section_label, \
          section_metadata_json, sort_order) \
         VALUES ($1, $2, $3, 'public', 'section-0', 'section-0', '{}'::jsonb, 0)",
    )
    .bind(file_id)
    .bind(document_id)
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("link the document to its owning file");
    let stale_translation = Uuid::new_v4();
    let stale_translation_version = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.document_translation_versions \
         (id, document_id, target_locale, source_record_hash, provider_key, \
          provider_config_hash, translated_title, translated_body_text) \
         VALUES ($1, $2, 'fr', 'stale-source-hash', 'test', 'test', 'stale', 'stale')",
    )
    .bind(stale_translation_version)
    .bind(document_id)
    .execute(db.pool())
    .await
    .expect("seed the stale translation version");
    sqlx::query(
        "INSERT INTO context69.document_translation_chunks \
         (id, translation_id, document_id, target_locale, chunk_index, chunk_text) \
         VALUES ($1, $2, $3, 'fr', 0, 'stale translation')",
    )
    .bind(stale_translation)
    .bind(stale_translation_version)
    .bind(document_id)
    .execute(db.pool())
    .await
    .expect("seed the stale translation chunk");

    let old_originals = live_chunk_ids(&db, document_id).await;
    assert_eq!(old_originals.len(), 4);
    let mut retired_ids = old_originals.clone();
    retired_ids.push(stale_translation);

    let row = db
        .list_rebuild_documents_page(Some(document_id - 1), 10)
        .await
        .expect("read the rebuild document page")
        .into_iter()
        .find(|row| row.document_id == document_id)
        .expect("the seeded document is pageable")
        .clone();
    let fresh: Vec<context69::domain::DocumentChunk> = (0..6)
        .map(|index| context69::domain::DocumentChunk {
            id: Uuid::new_v4(),
            document_id,
            chunk_index: index,
            text: format!("replacement chunk {index}"),
            record_hash: row.record_hash.clone(),
        })
        .collect();
    let fresh_ids: Vec<Uuid> = fresh.iter().map(|chunk| chunk.id).collect();

    let run_id = Uuid::new_v4();
    let entry = context69_search::SearchRegistryEntry {
        run_id,
        file_id: Some(file_id),
        document_ids: vec![document_id],
        staged_ids: fresh_ids.clone(),
        retired_ids: Vec::new(),
        baseline_record_hash: None,
        staged_record_hash: None,
        generation: 13,
        phase: "staging".into(),
    };
    let mut tx = db.pool().begin().await.expect("begin the producer switch");
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &entry,
            Some(&desired),
            i64::try_from(retired_ids.len()).unwrap(),
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("admit the replacement staging entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    let mut connection = tx;
    db.delete_document_chunks_in(&mut connection, document_id)
        .await
        .expect("delete the old original chunks");
    db.insert_document_chunks_in(&mut connection, document_id, &row.record_hash, &fresh)
        .await
        .expect("insert the replacement chunks");
    assert!(
        db.switch_search_registry_entry_with_retired_origin_in(
            run_id,
            &retired_ids,
            &old_originals,
            &mut connection,
        )
        .await
        .expect("switch the replacement entry with its origin")
        .is_some()
    );
    connection
        .commit()
        .await
        .expect("commit the producer switch");
    let entry = db
        .read_search_registry_state()
        .await
        .expect("read the switched entry")
        .entries
        .into_iter()
        .find(|entry| entry.run_id == run_id)
        .expect("the switched replacement entry");
    assert_eq!(entry.phase, "switched");
    assert!(entry.staged_ids.is_empty());
    let mut stored_retired = entry.retired_ids.clone();
    stored_retired.sort();
    let mut expected_retired = retired_ids.clone();
    expected_retired.sort();
    assert_eq!(stored_retired, expected_retired);
    let origin = sqlx::query_file!("tests/sql/vector_registry/read_retired_origin.sql", run_id)
        .fetch_one(db.pool())
        .await
        .expect("read the producer provenance");
    let mut stored_origin = origin.retired_original_ids.clone();
    stored_origin.sort();
    let mut expected_origin = old_originals.clone();
    expected_origin.sort();
    assert_eq!(stored_origin, expected_origin);
    assert_eq!(origin.reserved_ids, 5);

    let mut connection = db.pool().acquire().await.expect("acquire current usage");
    let before = sqlx::query_file!(
        "src/sql/db/search_registry/read_usage.sql",
        file_id,
        &Vec::<Uuid>::new(),
    )
    .fetch_one(&mut *connection)
    .await
    .expect("read the C1 usage");
    assert_eq!(before.total, 5, "the active C1 row counts all five ids");
    assert_eq!(before.file_total, 5);
    drop(connection);

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the rebuild claim");
    let (outcome, _) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            Some(13),
        )
        .await
        .expect("claim the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the rebuild claim");
    assert!(
        db.stage_rebuild_document(
            TARGET,
            &row,
            &fresh,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
        )
        .await
        .expect("stage the target chunks")
    );
    let counts = db
        .count_rebuild_staging_chunks(TARGET)
        .await
        .expect("count the staged snapshot");
    let total = counts.original + counts.translated;
    let mut connection = db.pool().acquire().await.expect("acquire completion");
    assert!(
        db.complete_rebuild_staging_in(
            &mut connection,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            total,
        )
        .await
        .expect("complete the staged snapshot")
    );
    drop(connection);

    sqlx::query(
        "UPDATE context69.runtime_file_library_settings \
         SET registry_max_ids_total = 6, registry_max_ids_per_file = 6, \
             registry_max_ids_per_run = 6 \
         WHERE singleton = TRUE",
    )
    .execute(db.pool())
    .await
    .expect("narrow the registry capacity policy");
    let state = context69::db::VectorIndexState {
        collection_name: TARGET,
        fingerprint: &desired.embedding_fingerprint,
        embedding_base_url: "https://embeddings.example/v1",
        embedding_model: "model-a",
        dimensions: 8,
        rebuilt_chunks: fresh.len(),
    };
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the refused activation");
    let error = db
        .complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
        .await
        .expect_err("a projection of seven ids refuses the cap of six");
    assert!(
        error.to_string().contains("capacity policy"),
        "the refusal names the capacity policy: {error}"
    );
    tx.rollback()
        .await
        .expect("roll back the refused activation");
    assert_eq!(
        db.read_active_vector_index_state()
            .await
            .expect("read the active vector index")
            .expect("the previous collection is still active")
            .collection_name,
        COLLECTION
    );
    assert_eq!(
        live_chunk_ids(&db, document_id).await,
        fresh_ids,
        "the refused activation keeps the replacement chunks"
    );

    sqlx::query(
        "UPDATE context69.runtime_file_library_settings \
         SET registry_max_ids_total = 7, registry_max_ids_per_file = 7, \
             registry_max_ids_per_run = 7 \
         WHERE singleton = TRUE",
    )
    .execute(db.pool())
    .await
    .expect("widen the registry capacity policy");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the accepted activation");
    assert!(
        db.complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
            .await
            .expect("the discounted projection fits the policy")
            .is_some()
    );
    tx.commit().await.expect("commit the accepted activation");

    let state = db
        .read_search_registry_state()
        .await
        .expect("read the registry state");
    let entry = state
        .entries
        .iter()
        .find(|entry| entry.run_id == run_id)
        .expect("the replacement entry stays as history");
    assert_eq!(entry.retired_ids.len(), 5);
    let origin = sqlx::query_file!("tests/sql/vector_registry/read_retired_origin.sql", run_id)
        .fetch_one(db.pool())
        .await
        .expect("read the preserved provenance");
    assert_eq!(origin.retired_original_ids, old_originals);
    assert_eq!(origin.reserved_ids, 5);
    let mut connection = db.pool().acquire().await.expect("acquire the projection");
    let projected = db
        .read_projected_registry_usage_in(&mut connection, TARGET)
        .await
        .expect("read the projected usage");
    assert_eq!(projected.total, 7);
    assert_eq!(projected.max_file, 7);
    assert_eq!(projected.max_run, 6);
    drop(connection);
    let usage = sqlx::query_file!(
        "src/sql/db/search_registry/read_usage.sql",
        file_id,
        &Vec::<Uuid>::new(),
    )
    .fetch_one(db.pool())
    .await
    .expect("read the C2 usage");
    assert_eq!(usage.total, 7);
    assert_eq!(usage.file_total, 7);
    assert_eq!(
        live_chunk_ids(&db, document_id).await,
        fresh.iter().map(|chunk| chunk.id).collect::<Vec<_>>()
    );

    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("remove the fixture library file");
    reset_acceptance_fixture(&db, group_id, &[COLLECTION, TARGET]).await;
}

/// A legacy entry without provenance, a translation-only entry, an abandoned
/// entry and an id-less live reservation all stay fully reserved, and a tight
/// cap refuses an activation that would depend on an unproven discount.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_quota_projection_keeps_unproven_reservations_and_refuses_a_legacy_overflow() {
    const TARGET: &str = "barrier-quota-legacy__target";
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
    let group_id = seed_barrier_fixture_group(&db).await;
    let (document_id, _) = support::seed_reindex_document(&db, group_id, 3)
        .await
        .expect("seed the canonical document");
    let row = db
        .list_rebuild_documents_page(Some(document_id - 1), 10)
        .await
        .expect("read the rebuild document page")
        .into_iter()
        .find(|row| row.document_id == document_id)
        .expect("the seeded document is pageable")
        .clone();
    let fresh: Vec<context69::domain::DocumentChunk> = (0..3)
        .map(|index| context69::domain::DocumentChunk {
            id: Uuid::new_v4(),
            document_id,
            chunk_index: index,
            text: format!("legacy fixture chunk {index}"),
            record_hash: row.record_hash.clone(),
        })
        .collect();

    let legacy_run = Uuid::new_v4();
    let legacy_ids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
    sqlx::query(
        "INSERT INTO context69.search_registry_entries \
         (run_id, file_id, document_ids, staged_ids, retired_ids, reserved_ids, generation, \
          phase, embedding_fingerprint, collection_name) \
         VALUES ($1, NULL, '{}'::BIGINT[], '{}'::UUID[], $2, $3, 1, 'switched', $4, $5)",
    )
    .bind(legacy_run)
    .bind(&legacy_ids)
    .bind(i64::try_from(legacy_ids.len()).unwrap())
    .bind(&desired.embedding_fingerprint)
    .bind(COLLECTION)
    .execute(db.pool())
    .await
    .expect("seed the legacy switched entry");

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the rebuild claim");
    let (outcome, _) = db
        .begin_rebuild_checked_with_budget_in(
            &mut tx,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            Some(13),
        )
        .await
        .expect("claim the rebuild");
    assert!(matches!(outcome, RebuildStartOutcome::Started(_)));
    tx.commit().await.expect("commit the rebuild claim");
    assert!(
        db.stage_rebuild_document(
            TARGET,
            &row,
            &fresh,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
        )
        .await
        .expect("stage the target chunks")
    );
    let counts = db
        .count_rebuild_staging_chunks(TARGET)
        .await
        .expect("count the staged snapshot");
    let mut connection = db.pool().acquire().await.expect("acquire completion");
    assert!(
        db.complete_rebuild_staging_in(
            &mut connection,
            owner,
            owner_token,
            &desired.embedding_fingerprint,
            TARGET,
            counts.original + counts.translated,
        )
        .await
        .expect("complete the staged snapshot")
    );
    drop(connection);

    let state = context69::db::VectorIndexState {
        collection_name: TARGET,
        fingerprint: &desired.embedding_fingerprint,
        embedding_base_url: "https://embeddings.example/v1",
        embedding_model: "model-a",
        dimensions: 8,
        rebuilt_chunks: fresh.len(),
    };
    sqlx::query(
        "UPDATE context69.runtime_file_library_settings \
         SET registry_max_ids_total = 6, registry_max_ids_per_file = 6, \
             registry_max_ids_per_run = 6 \
         WHERE singleton = TRUE",
    )
    .execute(db.pool())
    .await
    .expect("narrow the registry capacity policy");
    let mut tx = db.pool().begin().await.expect("begin the legacy refusal");
    let error = db
        .complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
        .await
        .expect_err("an unproven legacy reservation cannot be discounted to fit");
    assert!(
        error.to_string().contains("capacity policy"),
        "the refusal names the capacity policy: {error}"
    );
    tx.rollback().await.expect("roll back the legacy refusal");
    assert_eq!(
        db.read_active_vector_index_state()
            .await
            .expect("read the active vector index")
            .expect("the previous collection is still active")
            .collection_name,
        COLLECTION
    );

    sqlx::query(
        "UPDATE context69.runtime_file_library_settings \
         SET registry_max_ids_total = 7, registry_max_ids_per_file = 7, \
             registry_max_ids_per_run = 7 \
         WHERE singleton = TRUE",
    )
    .execute(db.pool())
    .await
    .expect("widen the registry capacity policy");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the accepted activation");
    assert!(
        db.complete_owned_rebuild_in(&mut tx, owner, owner_token, &state)
            .await
            .expect("the fully counted legacy row fits the wider policy")
            .is_some()
    );
    tx.commit().await.expect("commit the accepted activation");
    let legacy = sqlx::query_file!(
        "tests/sql/vector_registry/read_retired_origin.sql",
        legacy_run
    )
    .fetch_one(db.pool())
    .await
    .expect("read the legacy entry");
    assert!(legacy.retired_original_ids.is_empty());
    assert_eq!(legacy.reserved_ids, 4);

    let translation_run = Uuid::new_v4();
    let translation_ids: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
    let abandoned_run = Uuid::new_v4();
    let abandoned_ids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
    let same_hash_run = Uuid::new_v4();
    let same_hash_ids: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
    let null_collection_run = Uuid::new_v4();
    let null_collection_ids: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
    let idless_run = Uuid::new_v4();
    let proven_run = Uuid::new_v4();
    let proven_origin: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
    let proven_retired: Vec<Uuid> = proven_origin
        .iter()
        .copied()
        .chain(std::iter::once(Uuid::new_v4()))
        .collect();
    for (run_id, retired_ids, phase, collection) in [
        (
            translation_run,
            &translation_ids,
            "switched",
            Some(COLLECTION),
        ),
        (abandoned_run, &abandoned_ids, "retired", Some(COLLECTION)),
        (same_hash_run, &same_hash_ids, "switched", Some(COLLECTION)),
        (null_collection_run, &null_collection_ids, "switched", None),
    ] {
        sqlx::query(
            "INSERT INTO context69.search_registry_entries \
             (run_id, file_id, document_ids, staged_ids, retired_ids, reserved_ids, generation, \
              phase, embedding_fingerprint, collection_name) \
             VALUES ($1, NULL, '{}'::BIGINT[], '{}'::UUID[], $2, $3, 1, $4, $5, $6)",
        )
        .bind(run_id)
        .bind(retired_ids)
        .bind(i64::try_from(retired_ids.len()).unwrap())
        .bind(phase)
        .bind(collection.map(|_| desired.embedding_fingerprint.as_str()))
        .bind(collection)
        .execute(db.pool())
        .await
        .expect("seed a fully reserved branch entry");
    }
    sqlx::query(
        "INSERT INTO context69.search_registry_entries \
         (run_id, file_id, document_ids, staged_ids, retired_ids, reserved_ids, generation, \
          phase, embedding_fingerprint, collection_name) \
         VALUES ($1, NULL, '{}'::BIGINT[], '{}'::UUID[], '{}'::UUID[], 5, 1, 'switched', $2, $3)",
    )
    .bind(idless_run)
    .bind(&desired.embedding_fingerprint)
    .bind(TARGET)
    .execute(db.pool())
    .await
    .expect("seed the id-less live original reservation");
    sqlx::query(
        "INSERT INTO context69.search_registry_entries \
         (run_id, file_id, document_ids, staged_ids, retired_ids, retired_original_ids, \
          reserved_ids, generation, phase, embedding_fingerprint, collection_name) \
         VALUES ($1, NULL, '{}'::BIGINT[], '{}'::UUID[], $2, $3, $4, 1, 'switched', $5, $6)",
    )
    .bind(proven_run)
    .bind(&proven_retired)
    .bind(&proven_origin)
    .bind(i64::try_from(proven_retired.len()).unwrap())
    .bind(&desired.embedding_fingerprint)
    .bind(COLLECTION)
    .execute(db.pool())
    .await
    .expect("seed the proven replacement entry");

    let usage = sqlx::query_file!(
        "src/sql/db/search_registry/read_usage.sql",
        Uuid::nil(),
        &Vec::<Uuid>::new(),
    )
    .fetch_one(db.pool())
    .await
    .expect("read the branch usage");
    assert_eq!(
        usage.total, 22,
        "legacy, translation-only, abandoned, same-hash, NULL-collection and id-less rows stay full; the proven origin is discounted"
    );
    let mut connection = db.pool().acquire().await.expect("acquire the projection");
    let projected = db
        .read_projected_registry_usage_in(&mut connection, TARGET)
        .await
        .expect("read the target projection");
    assert_eq!(projected.total, 22);
    let projected_old = db
        .read_projected_registry_usage_in(&mut connection, COLLECTION)
        .await
        .expect("read the old-collection projection");
    assert_eq!(
        projected_old.total, 24,
        "the proven entry counts fully while its own collection is the target"
    );
    drop(connection);

    reset_acceptance_fixture(&db, group_id, &[COLLECTION, TARGET]).await;
}

/// An ordinary writer is refused by the barrier and by a captured identity
/// that no longer names the active index; a missing captured identity fails
/// closed while the deployment publishes vectors.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unmanaged_writer_is_refused_by_the_barrier_and_by_a_lost_identity() {
    const TARGET: &str = "barrier-writer__target";
    let db = connect().await;
    reset_barrier(&db).await;
    clear_registry_entries(&db).await;
    clear_runtime_settings(&db).await;
    sqlx::query("DELETE FROM context69.vector_index_state")
        .execute(db.pool())
        .await
        .expect("clear the fixture vector index rows");

    let mut tx = db.pool().begin().await.expect("begin the unmanaged guard");
    db.require_idle_admissible_vector_identity_in(&mut tx, None)
        .await
        .expect("an unpublished deployment admits an unmanaged writer");
    tx.rollback().await.expect("roll back the unmanaged guard");

    let desired = seed_runtime_settings(
        &db,
        "http://qdrant.example:6333",
        COLLECTION,
        "https://embeddings.example/v1",
        "model-a",
        8,
    )
    .await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the lost identity guard");
    let error = db
        .require_idle_admissible_vector_identity_in(&mut tx, None)
        .await
        .expect_err("a lost identity fails closed while vectors are published");
    assert!(
        error.to_string().contains("captured none"),
        "the conflict is actionable: {error}"
    );
    tx.rollback()
        .await
        .expect("roll back the lost identity guard");

    let mut tx = db.pool().begin().await.expect("begin the matching guard");
    db.require_idle_admissible_vector_identity_in(&mut tx, Some(&desired))
        .await
        .expect("the captured identity names the active index");
    tx.rollback().await.expect("roll back the matching guard");

    let (owner, owner_token) = seed_task_item(&db, "running").await;
    let mut tx = db.pool().begin().await.expect("begin the rebuild claim");
    db.begin_rebuild_in(
        &mut tx,
        owner,
        owner_token,
        &desired.embedding_fingerprint,
        TARGET,
    )
    .await
    .expect("claim the rebuild");
    tx.commit().await.expect("commit the rebuild claim");
    support::pin_rebuild_snapshot(&db, &pinned_budget(), 1).await;
    support::mark_rebuild_staging_complete(&db).await;
    let mut tx = db.pool().begin().await.expect("begin the barrier guard");
    let error = db
        .require_idle_admissible_vector_identity_in(&mut tx, Some(&desired))
        .await
        .expect_err("a running rebuild refuses the ordinary writer");
    assert!(
        error.to_string().contains("rebuild is in progress"),
        "the conflict is actionable: {error}"
    );
    tx.rollback().await.expect("roll back the barrier guard");

    let mut tx = db.pool().begin().await.expect("begin the activation");
    assert!(
        db.complete_owned_rebuild_in(
            &mut tx,
            owner,
            owner_token,
            &context69::db::VectorIndexState {
                collection_name: TARGET,
                fingerprint: &desired.embedding_fingerprint,
                embedding_base_url: "https://embeddings.example/v1",
                embedding_model: "model-a",
                dimensions: 8,
                rebuilt_chunks: 0,
            },
        )
        .await
        .expect("activate the target")
        .is_some()
    );
    tx.commit().await.expect("commit the activation");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin the stale identity guard");
    let error = db
        .require_idle_admissible_vector_identity_in(&mut tx, Some(&desired))
        .await
        .expect_err("an identity captured before activation cannot write after it");
    assert!(
        error.to_string().contains("captured vector identity"),
        "the conflict is actionable: {error}"
    );
    tx.rollback()
        .await
        .expect("roll back the stale identity guard");

    sqlx::query("DELETE FROM context69.vector_index_state WHERE collection_name = ANY($1)")
        .bind(&[COLLECTION.to_string(), TARGET.to_string()])
        .execute(db.pool())
        .await
        .expect("delete the fixture vector index rows");
    clear_runtime_settings(&db).await;
    reset_barrier(&db).await;
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL and the designated test Qdrant"]
async fn an_actual_rebuild_panic_after_the_collection_exists_retains_the_panic_and_cleans_up() {
    let db = connect().await;
    let error = run_actual_rebuild_case(&db, RebuildCaseFault::PanicAfterCollection)
        .await
        .expect_err("the injected panic is reported after cleanup, not swallowed");
    assert!(
        format!("{error:#}").contains("injected post-collection panic"),
        "the original panic diagnostic is preserved: {error:#}"
    );
    let preflight = support::read_rebuild_fixture_preflight(&db)
        .await
        .expect("read the post-panic fixture");
    assert_eq!(
        preflight.emptiness_problem(),
        None,
        "the post-collection panic left no owned fixture behind"
    );
    let collections = support::pause_collections("http://unraid.cloud1ful.com:6336")
        .await
        .expect("list the post-panic collections");
    assert!(
        collections.is_empty(),
        "the post-collection panic left no owned collection: {collections:?}"
    );
}
