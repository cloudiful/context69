//! Durable physical-deletion recovery for released sources: fault injection,
//! restart recovery, reuse-before-cleanup identity protection, and legacy
//! direct-path retry.

use super::support::*;
use super::support_seed_release::*;

#[tokio::test]
async fn physical_delete_failure_is_rescheduled_then_retried() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"retry me".to_vec();
    let sha = "c1".repeat(32);
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
        .expect("release source");
    let first = service
        .run_source_object_cleanup(50)
        .await
        .expect("failing cleanup pass");
    assert!(
        first.failed >= 1,
        "the injected failure must be rescheduled"
    );
    assert!(
        storage_root.join(&key).exists(),
        "failed physical delete must keep the bytes"
    );
    assert!(
        object_row_exists(&db, object_id).await,
        "object metadata must be preserved until deletion is confirmed"
    );
    assert_eq!(open_cleanup_intents(&db, object_id).await, 1);
    assert!(cleanup_intent_attempts(&db, object_id).await >= 1);
    drop(failpoint);

    force_cleanup_due(&db, object_id).await;
    let second = service
        .run_source_object_cleanup(50)
        .await
        .expect("recovery cleanup pass");
    assert!(second.deleted >= 1);
    assert!(!storage_root.join(&key).exists());
    assert!(!object_row_exists(&db, object_id).await);
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn restart_recovery_drains_pending_intent() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"restart me".to_vec();
    let sha = "c2".repeat(32);
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
        .expect("release source");
    let failed = service
        .run_source_object_cleanup(50)
        .await
        .expect("failing cleanup pass");
    assert!(failed.failed >= 1);
    drop(failpoint);

    // A fresh service over the same database and storage root models a
    // process restart; only the durable intent can drive recovery.
    let restarted = build_library_service_at(&db, storage_root.clone()).await;
    force_cleanup_due(&db, object_id).await;
    let recovered = restarted
        .run_source_object_cleanup(50)
        .await
        .expect("restart cleanup pass");
    assert!(recovered.deleted >= 1);
    assert!(!storage_root.join(&key).exists());
    assert!(!object_row_exists(&db, object_id).await);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn reuse_before_cleanup_cancels_intent_without_deleting_bytes() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"reuse me".to_vec();
    let sha = "c3".repeat(32);
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
        .expect("release source");
    let failed = service
        .run_source_object_cleanup(50)
        .await
        .expect("failing cleanup pass");
    assert!(failed.failed >= 1);
    drop(failpoint);

    // A new file starts referencing the same object before cleanup runs. The
    // identity guard must cancel the intent instead of deleting the bytes.
    let reused = seed_file_referencing_object(&db, group.id, object_id, &key, &sha).await;
    force_cleanup_due(&db, object_id).await;
    let cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup after reuse");
    assert_eq!(cleanup.deleted, 0, "reused object must not be deleted");
    assert!(
        storage_root.join(&key).exists(),
        "reused bytes must survive"
    );
    assert!(object_row_exists(&db, object_id).await);
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);

    let reused_detail = service
        .get_file_in_project(&group, reused)
        .await
        .expect("reused file detail");
    assert!(
        reused_detail.source_available,
        "the reusing file must still see its source"
    );
    let released_detail = service
        .get_file_in_project(&group, file_id)
        .await
        .expect("released file detail");
    assert!(!released_detail.source_available);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn already_missing_bytes_complete_the_intent() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"partial".to_vec();
    let sha = "c4".repeat(32);
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
        .expect("release source");

    // Model a crash after the physical delete but before the row delete: the
    // retry must skip the missing bytes and finish the row removal.
    std::fs::remove_file(storage_root.join(&key)).expect("remove bytes");
    let cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup pass");
    assert!(cleanup.deleted >= 1);
    assert!(!object_row_exists(&db, object_id).await);
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn superseded_object_identity_is_not_deleted() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"supersede".to_vec();
    let sha = "c5".repeat(32);
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
        .expect("release source");

    // The original object row disappears and a fresh object with the same key
    // (and same group+sha) appears before cleanup runs. The stale intent must
    // not delete the new bytes.
    sqlx::query("DELETE FROM context69.library_storage_objects WHERE id = $1")
        .bind(object_id)
        .execute(db.pool())
        .await
        .expect("delete original object row");
    let (replacement_id, replacement_key) =
        seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    seed_file_referencing_object(&db, group.id, replacement_id, &replacement_key, &sha).await;

    let cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup pass");
    assert_eq!(
        cleanup.deleted, 0,
        "a superseded identity must not be deleted"
    );
    assert_eq!(cleanup.cancelled, 1);
    assert!(object_row_exists(&db, replacement_id).await);
    assert!(storage_root.join(&replacement_key).exists());
    assert_eq!(replacement_key, key, "content key must be reused");

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn legacy_direct_path_delete_failure_is_retried() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    purge_open_cleanup_intents(&db).await;
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let path = format!("legacy/outbox-source-{}", uuid::Uuid::new_v4());
    write_object(&storage_root, &path, b"legacy bytes");
    let file_id = seed_legacy_file(&db, group.id, &path, false).await;

    let failpoint = DeleteFailpoint::enabled();
    service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("release legacy source");
    let failed = service
        .run_source_object_cleanup(50)
        .await
        .expect("failing cleanup pass");
    assert!(failed.failed >= 1);
    assert!(storage_root.join(&path).exists());
    assert_eq!(open_cleanup_intents_for_path(&db, &path).await, 1);
    drop(failpoint);

    force_cleanup_due_for_path(&db, &path).await;
    let recovered = service
        .run_source_object_cleanup(50)
        .await
        .expect("legacy recovery pass");
    assert!(recovered.deleted >= 1);
    assert!(!storage_root.join(&path).exists());

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
