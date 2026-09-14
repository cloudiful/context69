//! Issue 389 dispatcher integration: wake-after-commit, fallback, shutdown,
//! rollback isolation, immediate cleanup, retry, restart drain, duplicate
//! release, and shared-object races.
//!
//! DB-backed cases run only when `CONTEXT69_TEST_DATABASE_URL` points at a
//! scratch database; they are skipped otherwise like the existing
//! `library_source_release` suite.

#[allow(dead_code)]
#[path = "library_source_release/support.rs"]
mod support;

#[path = "library_source_release/support_seed.rs"]
mod support_seed;

use std::time::Duration;

use support::*;
use tokio_util::sync::CancellationToken;

/// Wire a dispatcher onto a freshly built service so release call sites wake
/// the same `Notify` the background loop drains on.
async fn build_wired_service(
    db: &context69::db::Database,
    fallback: Duration,
) -> (
    context69::services::library::LibraryService,
    std::path::PathBuf,
    context69::services::library::SourceCleanupDispatcher,
) {
    let (mut service, storage_root) = build_library_service(db).await;
    let dispatcher =
        context69::services::library::SourceCleanupDispatcher::with_fallback_interval(fallback);
    service.set_source_cleanup_dispatcher(dispatcher.clone());
    (service, storage_root, dispatcher)
}

#[tokio::test]
async fn wake_drains_immediately_without_waiting_for_fallback() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    // Hour-long fallback proves the drain below comes from the wake, not the
    // periodic pass.
    let (service, storage_root, dispatcher) =
        build_wired_service(&db, Duration::from_secs(3600)).await;
    let group = seed_group_record(&db).await;
    let content = b"wake me".to_vec();
    let sha = "e1".repeat(32);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    let shutdown = CancellationToken::new();
    let loop_service = service.clone();
    let loop_dispatcher = dispatcher.clone();
    let loop_shutdown = shutdown.clone();
    let loop_handle = tokio::spawn(async move {
        loop_dispatcher.run(loop_service, loop_shutdown).await;
    });
    // Give the startup drain a moment to settle with no work.
    tokio::time::sleep(Duration::from_millis(100)).await;

    service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("manual release commits and wakes without waiting for S3");
    // Release returns after commit + wake; the background drain must delete
    // promptly even though the fallback is an hour away.
    let mut deleted = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if !storage_root.join(&key).exists() && !object_row_exists(&db, object_id).await {
            deleted = true;
            break;
        }
    }
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;

    assert!(
        deleted,
        "wake after commit must drain immediately, not after the fallback"
    );
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn fallback_drains_without_any_wake() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root, dispatcher) =
        build_wired_service(&db, Duration::from_millis(50)).await;
    let group = seed_group_record(&db).await;
    let content = b"fallback me".to_vec();
    let sha = "e2".repeat(32);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("release");
    // Drop the wake: even if the notification were lost, the short fallback
    // must still drain. We cannot un-send the wake, so this case seeds the
    // intent via direct SQL to model a crash between commit and wake.
    purge_open_cleanup_intents(&db).await;
    sqlx::query("UPDATE context69.library_files SET source_released_at = now() WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("mark released");
    sqlx::query(
        "INSERT INTO context69.library_storage_object_cleanup \
         (object_id, group_id, sha256, object_key, storage_backend) \
         VALUES ($1, $2, $3, $4, 'local')",
    )
    .bind(object_id)
    .bind(group.id)
    .bind(&sha)
    .bind(&key)
    .execute(db.pool())
    .await
    .expect("recreate intent without wake");

    let shutdown = CancellationToken::new();
    let loop_service = service.clone();
    let loop_dispatcher = dispatcher.clone();
    let loop_shutdown = shutdown.clone();
    let loop_handle = tokio::spawn(async move {
        loop_dispatcher.run(loop_service, loop_shutdown).await;
    });
    let mut deleted = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if !storage_root.join(&key).exists() {
            deleted = true;
            break;
        }
    }
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;
    assert!(
        deleted,
        "fallback must drain an intent that never received a wake"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn dispatcher_shutdown_exits_promptly() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root, dispatcher) =
        build_wired_service(&db, Duration::from_secs(3600)).await;
    let shutdown = CancellationToken::new();
    let loop_shutdown = shutdown.clone();
    let loop_service = service.clone();
    let loop_dispatcher = dispatcher.clone();
    let handle = tokio::spawn(async move {
        loop_dispatcher.run(loop_service, loop_shutdown).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("shutdown must stop the dispatcher")
        .expect("dispatcher task");
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn release_rollback_creates_no_intent_and_needs_no_wake() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root, _dispatcher) =
        build_wired_service(&db, Duration::from_secs(3600)).await;
    let group = seed_group_record(&db).await;
    let content = b"not succeeded".to_vec();
    let sha = "e3".repeat(32);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "queued".to_string(),
            delete_source_after_processing: false,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    let result = service
        .release_file_source_in_project(&group, file_id)
        .await;
    assert!(
        result.is_err(),
        "non-succeeded release must roll back, not wake"
    );
    assert_eq!(
        open_cleanup_intents(&db, object_id).await,
        0,
        "rollback must leave no observable cleanup intent"
    );
    assert!(
        storage_root.join(&key).exists(),
        "rollback must keep the bytes"
    );
    assert!(object_row_exists(&db, object_id).await);
    // No wake was sent, so a drain must find nothing to do.
    let summary = service
        .run_source_object_cleanup(50)
        .await
        .expect("drain after rollback");
    assert_eq!(summary.scanned, 0);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn duplicate_release_with_dispatcher_stays_idempotent() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root, _dispatcher) =
        build_wired_service(&db, Duration::from_secs(3600)).await;
    let group = seed_group_record(&db).await;
    let content = b"dup wake".to_vec();
    let sha = "e4".repeat(32);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("first release");
    assert_eq!(open_cleanup_intents(&db, object_id).await, 1);
    service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("second release stays idempotent and sends no extra wake");
    assert_eq!(
        open_cleanup_intents(&db, object_id).await,
        1,
        "duplicate release must not duplicate the intent"
    );
    let cleanup = service.run_source_object_cleanup(50).await.expect("drain");
    assert!(cleanup.deleted >= 1);
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn failure_retry_and_restart_drain_recover() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root, _dispatcher) =
        build_wired_service(&db, Duration::from_secs(3600)).await;
    let group = seed_group_record(&db).await;
    let content = b"retry restart".to_vec();
    let sha = "e5".repeat(32);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    let failpoint = DeleteFailpoint::enabled();
    service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("release");
    let failed = service
        .run_source_object_cleanup(50)
        .await
        .expect("failing drain");
    assert!(failed.failed >= 1);
    assert!(storage_root.join(&key).exists());
    drop(failpoint);

    // Fresh service over the same rows models a restart: the durable intent
    // alone must drive recovery via startup/fallback drain.
    let restarted = build_library_service_at(&db, storage_root.clone()).await;
    force_cleanup_due(&db, object_id).await;
    let recovered = restarted
        .run_source_object_cleanup(50)
        .await
        .expect("restart drain");
    assert!(recovered.deleted >= 1);
    assert!(!storage_root.join(&key).exists());
    assert!(!object_row_exists(&db, object_id).await);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn shared_object_and_new_reference_race_never_deletes_bytes() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root, _dispatcher) =
        build_wired_service(&db, Duration::from_secs(3600)).await;
    let group = seed_group_record(&db).await;
    let content = b"shared race".to_vec();
    let sha = "e6".repeat(32);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let first = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;
    let second = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    service
        .release_file_source_in_project(&group, first)
        .await
        .expect("release first");
    // A new reference landing before the drain must cancel, not delete.
    let _third = seed_file_referencing_object(&db, group.id, object_id, &key, &sha).await;
    let first_cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("drain with live references");
    assert_eq!(
        first_cleanup.deleted, 0,
        "shared or re-referenced bytes must survive"
    );
    assert!(storage_root.join(&key).exists());
    assert!(object_row_exists(&db, object_id).await);

    service
        .release_file_source_in_project(&group, second)
        .await
        .expect("release second");
    // Still referenced by the third file: still no delete.
    force_cleanup_due(&db, object_id).await;
    let second_cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("drain with remaining reference");
    assert_eq!(second_cleanup.deleted, 0);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
