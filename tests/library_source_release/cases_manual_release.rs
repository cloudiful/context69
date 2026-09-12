//! Manual source release: source-only deletion, result retention, idempotency,
//! and shared content-addressed object safety.

use super::support::*;

#[tokio::test]
async fn manual_release_deletes_source_and_keeps_results() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"source body".to_vec();
    let sha = "1".repeat(64);
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
    seed_file_document(&db, group.id, file_id).await;

    let released = service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("release source");
    assert!(
        !released.source_available,
        "released source must be unavailable"
    );
    assert_eq!(released.size_bytes, content.len() as i64);
    assert_eq!(released.ingest_status.as_str(), "succeeded");

    let cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup worker");
    assert!(
        cleanup.deleted >= 1,
        "cleanup worker must delete the object"
    );
    assert!(
        !storage_root.join(&key).exists(),
        "physical source must be gone"
    );
    assert!(
        !object_row_exists(&db, object_id).await,
        "unreferenced storage object row must be deleted"
    );
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);
    let (detached, released_at) = release_state(&db, file_id).await;
    assert!(detached, "released file must be detached from its object");
    assert!(released_at.is_some(), "deliberate release must be recorded");
    assert_eq!(
        file_document_count(&db, file_id).await,
        1,
        "processed full text must be retained"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn release_is_idempotent() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"idempotent".to_vec();
    let sha = "2".repeat(64);
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
    let (_, first_released_at) = release_state(&db, file_id).await;
    assert_eq!(open_cleanup_intents(&db, object_id).await, 1);
    let second = service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("second release must succeed idempotently");
    assert!(!second.source_available);
    let (_, second_released_at) = release_state(&db, file_id).await;
    assert_eq!(
        first_released_at, second_released_at,
        "second release must not change the release timestamp"
    );
    assert_eq!(
        open_cleanup_intents(&db, object_id).await,
        1,
        "repeated release must not duplicate the cleanup intent"
    );
    service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup worker");
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);
    service
        .run_source_object_cleanup(50)
        .await
        .expect("second cleanup pass");
    assert_eq!(
        completed_cleanup_intents(&db, object_id).await,
        1,
        "completed intent must not be reprocessed"
    );
    assert_eq!(open_cleanup_intents(&db, object_id).await, 0);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn shared_object_survives_until_last_release() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"shared bytes".to_vec();
    let sha = "3".repeat(64);
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
    let first_cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup after first release");
    assert!(first_cleanup.cancelled >= 1);
    assert_eq!(completed_cleanup_intents(&db, object_id).await, 1);
    assert!(
        object_row_exists(&db, object_id).await,
        "shared object row must survive while another file references it"
    );
    assert!(
        storage_root.join(&key).exists(),
        "shared physical bytes must survive the first release"
    );
    let second_detail = service
        .get_file_in_project(&group, second)
        .await
        .expect("second file detail");
    assert!(
        second_detail.source_available,
        "the other file must still see its source"
    );

    service
        .release_file_source_in_project(&group, second)
        .await
        .expect("release second");
    let second_cleanup = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup after second release");
    assert!(second_cleanup.deleted >= 1);
    assert_eq!(
        completed_cleanup_intents(&db, object_id).await,
        2,
        "each release records its own intent"
    );
    assert!(
        !object_row_exists(&db, object_id).await,
        "last release must delete the object row"
    );
    assert!(
        !storage_root.join(&key).exists(),
        "last release must delete the shared physical bytes"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
