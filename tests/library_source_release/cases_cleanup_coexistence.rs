//! Coexistence with the startup missing-source cleanup: a deliberate release
//! must not be selected as a missing source, so a released file's results are
//! never deleted by that path.

use context69::library_store::LibraryStore;

use super::support::*;

#[tokio::test]
async fn snapshot_restore_reinstates_the_release_marker() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let group = seed_group_record(&db).await;
    let file_id = seed_legacy_file(&db, group.id, "legacy/restore-source", true).await;
    let store = LibraryStore::new(db.clone());
    let record = store
        .get_file(file_id)
        .await
        .expect("load file")
        .expect("file exists");
    let released_at = release_state(&db, file_id).await.1.expect("released_at");

    // A content update clears the marker; a rollback must reinstate it so the
    // missing-source cleanup still treats the row as deliberately released.
    sqlx::query("UPDATE context69.library_files SET source_released_at = NULL WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clear marker");
    store
        .restore_file_snapshot_in_project(&record, None, Some(released_at))
        .await
        .expect("restore snapshot")
        .expect("file restored");
    assert_eq!(
        release_state(&db, file_id).await.1,
        Some(released_at),
        "rollback must reinstate the deliberate release marker"
    );

    cleanup_group(&db, group.id).await;
}

#[tokio::test]
async fn missing_source_selection_excludes_deliberately_released_rows() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let group = seed_group_record(&db).await;
    let released = seed_legacy_file(&db, group.id, "legacy/released-source", true).await;
    let missing = seed_legacy_file(&db, group.id, "legacy/missing-source", false).await;
    let store = LibraryStore::new(db.clone());

    // grace = -1h makes every seeded row old enough to be a candidate.
    let page = store
        .list_missing_legacy_source_files(-1, None, None, 100)
        .await
        .expect("select missing-source candidates");
    let ids = page.iter().map(|row| row.id).collect::<Vec<_>>();
    assert!(
        ids.contains(&missing),
        "an incidentally missing source stays a cleanup candidate"
    );
    assert!(
        !ids.contains(&released),
        "a deliberately released row must never be a missing-source candidate"
    );

    cleanup_group(&db, group.id).await;
}

#[tokio::test]
async fn released_file_row_and_documents_survive_missing_source_path() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"released".to_vec();
    let sha = "a1".repeat(32);
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
            storage_rel_path: OrphanKey::Object(key),
        },
    )
    .await;
    seed_file_document(&db, group.id, file_id).await;
    service
        .release_file_source_in_project(&group, file_id)
        .await
        .expect("release source");

    // The row is now detached and released; it must not look like a legacy
    // missing source even though its bytes are gone.
    let store = LibraryStore::new(db.clone());
    let page = store
        .list_missing_legacy_source_files(-1, None, None, 100)
        .await
        .expect("select missing-source candidates");
    assert!(
        !page.iter().any(|row| row.id == file_id),
        "released content-addressed row must be excluded"
    );
    assert_eq!(
        file_document_count(&db, file_id).await,
        1,
        "released results must stay readable"
    );
    let row: String =
        sqlx::query_scalar("SELECT ingest_status FROM context69.library_files WHERE id = $1")
            .bind(file_id)
            .fetch_one(db.pool())
            .await
            .expect("load status");
    assert_eq!(row, "succeeded");

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
