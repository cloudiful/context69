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
    delete_tasks, fingerprint, identity, registry_entry, reset_barrier, seed_runtime_settings,
    seed_task_item, seed_task_item_full, set_item_lease,
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
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
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
        concurrency: 1,
    }))
}

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

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_common_pause_sets_the_barrier_flag_and_keeps_the_cursor() {
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
    clear_registry_entries(&db).await;
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
        support::seed_reindex_document(&db, gid, 65).await?;

        // The rebuild reindexes the whole stored corpus; the preflight above
        // proved it is the case-owned corpus, so the global page order is the
        // expected cursor.
        let cursor_row: Option<(i64, i32)> = sqlx::query_as(
            "SELECT document_id, chunk_index FROM context69.document_chunks \
             ORDER BY document_id, chunk_index LIMIT 1 OFFSET 63",
        )
        .fetch_optional(db.pool())
        .await?;
        expected_cursor = cursor_row.map(|(document_id, chunk_index)| RebuildCursor {
            document_id,
            chunk_index,
        });
        total_chunks = sqlx::query_scalar("SELECT count(*) FROM context69.document_chunks")
            .fetch_one(db.pool())
            .await?;

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
