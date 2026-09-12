//! Release guards: cross-group isolation, active-processing refusal, sync
//! control-file refusal, and "failed ingest retains its source".

use super::support::*;

#[tokio::test]
async fn cross_group_release_is_rejected() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let owner = seed_group_record(&db).await;
    let other = seed_group_record(&db).await;
    let content = b"cross group".to_vec();
    let sha = "4".repeat(64);
    let (object_id, key) = seed_storage_object(&db, &storage_root, owner.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        owner.id,
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

    let error = service
        .release_file_source_in_project(&other, file_id)
        .await
        .expect_err("foreign group release must fail");
    assert!(
        error.to_string().contains("unknown file"),
        "foreign file must be indistinguishable from missing: {error}"
    );
    assert!(
        storage_root.join(&key).exists(),
        "foreign release must not touch the source"
    );

    cleanup_group(&db, owner.id).await;
    cleanup_group(&db, other.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn active_processing_blocks_release() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let user_id = seed_user(&db).await;
    let content = b"active".to_vec();
    let sha = "5".repeat(64);
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
    seed_active_task_item(&db, user_id, group.id, file_id).await;

    let error = service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect_err("release with an active item must fail");
    assert!(
        error.to_string().contains("active processing"),
        "expected active-processing refusal: {error}"
    );
    assert!(
        storage_root.join(&key).exists(),
        "active processing must not delete the source"
    );

    cleanup_group(&db, group.id).await;
    cleanup_user(&db, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn sync_control_file_release_is_refused() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"{\"source\": true}".to_vec();
    let sha = "6".repeat(64);
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
    sqlx::query("UPDATE context69.library_files SET filename = 'source.json' WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("rename to control file");

    let error = service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect_err("control file release must fail");
    assert!(
        error.to_string().contains("control files"),
        "expected control-file refusal: {error}"
    );
    assert!(
        storage_root.join(&key).exists(),
        "control file source must be retained"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn failed_ingest_retains_source() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"failed body".to_vec();
    let sha = "7".repeat(64);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let file_id = seed_file(
        &db,
        group.id,
        SeedFileOptions {
            sha256: sha.clone(),
            content: content.clone(),
            ingest_status: "failed".to_string(),
            delete_source_after_processing: true,
            source_released: false,
            storage_object_id: Some(object_id),
            storage_rel_path: OrphanKey::Object(key.clone()),
        },
    )
    .await;

    let error = service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect_err("failed file release must fail");
    assert!(
        error.to_string().contains("not succeeded"),
        "expected not-succeeded refusal: {error}"
    );

    let summary = service
        .retry_pending_source_releases(50)
        .await
        .expect("auto sweep");
    assert_eq!(summary.released, 0, "failed files must not auto-release");
    assert!(
        storage_root.join(&key).exists(),
        "failed ingest must retain its source"
    );
    let (_, released_at) = release_state(&db, file_id).await;
    assert!(released_at.is_none());

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
