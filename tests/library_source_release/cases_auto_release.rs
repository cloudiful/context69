//! Upload-time opt-in auto-release: retry after a crash or a transient storage
//! failure, and the default "retain the source" behavior.

use super::support::*;

#[tokio::test]
async fn auto_release_sweep_releases_opted_in_succeeded_files() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"auto release".to_vec();
    let sha = "8".repeat(64);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: true,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    // This models a crash after the success commit but before the inline
    // auto-release: the persisted opt-in flag is the durable intent.
    let first = service
        .retry_pending_source_releases(50)
        .await
        .expect("first sweep");
    assert_eq!(first.released, 1, "opted-in succeeded file must release");
    let cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup worker");
    assert!(cleanup.deleted >= 1);
    assert!(
        !storage_root.join(&key).exists(),
        "source bytes must be gone"
    );
    assert!(!object_row_exists(&db, object_id).await);
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);
    let (detached, released_at) = release_state(&db, file_id).await;
    assert!(detached);
    assert!(released_at.is_some());

    let second = service
        .retry_pending_source_releases(50)
        .await
        .expect("second sweep");
    assert_eq!(second.released, 0, "already-released file must not retry");

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn default_upload_retains_source() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"retain me".to_vec();
    let sha = "9".repeat(64);
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

    let summary = service
        .retry_pending_source_releases(50)
        .await
        .expect("sweep");
    assert_eq!(summary.released, 0, "default policy must retain the source");
    assert!(storage_root.join(&key).exists());
    let (detached, released_at) = release_state(&db, file_id).await;
    assert!(!detached);
    assert!(released_at.is_none());

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
