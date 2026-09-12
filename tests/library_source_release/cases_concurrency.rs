//! Release must serialize against reprocess through the same per-file
//! advisory lock the create/retry/rerun paths take, and object cleanup must
//! hold the storage-object row lock that blocks a concurrent new reference.

use std::time::Duration;

use context69::library_store::LibraryStore;

use super::support::*;

/// If release forgot the shared lock it returns in milliseconds and the
/// assertion trips.
const LOCK_HOLD: Duration = Duration::from_millis(800);

#[tokio::test]
async fn new_reference_insert_waits_for_the_cleanup_object_lock() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"identity".to_vec();
    let sha = "d4".repeat(32);
    let (object_id, key) = seed_storage_object(&db, &storage_root, group.id, &sha, &content).await;
    let store = LibraryStore::new(db.clone());

    // Model the cleanup worker holding the identity lock for the object row.
    let mut lock_tx = db.pool().begin().await.expect("begin lock transaction");
    store
        .lock_cleanup_storage_object(&mut lock_tx, object_id)
        .await
        .expect("lock object")
        .expect("object exists");

    // A new file referencing the object must block on the FK's FOR KEY SHARE
    // lock until cleanup finishes, so bytes can never be deleted under a new
    // reference.
    let insert = tokio::spawn({
        let db = db.clone();
        let key = key.clone();
        let sha = sha.clone();
        async move { seed_file_referencing_object(&db, group.id, object_id, &key, &sha).await }
    });
    tokio::time::sleep(LOCK_HOLD).await;
    assert!(
        !insert.is_finished(),
        "a new storage-object reference must wait for the cleanup row lock"
    );
    lock_tx.rollback().await.expect("release object lock");
    let reused = insert.await.expect("join insert");

    let reused_detail = service
        .get_file_in_project(&group, reused)
        .await
        .expect("reused file detail");
    assert!(reused_detail.source_available);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn release_waits_for_the_shared_file_lock() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"lock race".to_vec();
    let sha = "b2".repeat(32);
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

    let mut lock_tx = db.pool().begin().await.expect("begin lock transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("library_file_processing:{file_id}"))
        .execute(&mut *lock_tx)
        .await
        .expect("hold file processing lock");

    let handle = tokio::spawn({
        let service = service.clone();
        let group = group.clone();
        async move {
            service
                .release_file_source_in_project(&group, file_id)
                .await
        }
    });
    tokio::time::sleep(LOCK_HOLD).await;
    assert!(
        !handle.is_finished(),
        "release must wait for the shared file processing lock"
    );
    lock_tx.rollback().await.expect("release lock transaction");

    let detail = handle
        .await
        .expect("join release")
        .expect("release after lock");
    assert!(!detail.source_available);
    service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup worker");
    assert!(!storage_root.join(&key).exists());

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
