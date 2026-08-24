//! Regression tests for the same-content reuse behavior in the library
//! service. The library stores bytes in a content-addressed storage object so
//! uploads of identical bytes must deduplicate while still creating a fresh
//! `library_files` row whenever the requested `external_id` differs from the
//! stored one. The shared storage object must outlive the deletion of either
//! file row, and true external-id conflicts must still fail.
//!
//! These tests run only when `CONTEXT69_TEST_DATABASE_URL` points to a
//! scratch database (migrations are applied automatically). They are skipped
//! otherwise.

use context69::db::Database;
use context69::library_store::{LibraryStore, NewLibraryFile};
use sqlx::Row;
use uuid::Uuid;

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_group(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups \
         (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("dup-content-{}", Uuid::new_v4()))
    .bind("Duplicate Content Test Group")
    .bind(format!("test/dup-content-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

async fn insert_storage_object(db: &Database, group_id: i64, sha256: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(group_id)
    .bind(sha256)
    .bind(64_i64)
    .bind("local")
    .bind(format!("objects/{group_id}/{sha256}"))
    .execute(db.pool())
    .await
    .expect("insert storage object");
    id
}

async fn count_files_for_storage_object(db: &Database, object_id: Uuid) -> i64 {
    let row = sqlx::query(
        "SELECT count(*) AS references \
         FROM context69.library_files WHERE storage_object_id = $1",
    )
    .bind(object_id)
    .fetch_one(db.pool())
    .await
    .expect("count file references");
    row.get("references")
}

async fn file_external_id(db: &Database, file_id: Uuid) -> Option<String> {
    let row = sqlx::query("SELECT external_id FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .fetch_one(db.pool())
        .await
        .expect("load file external_id");
    row.get("external_id")
}

async fn file_storage_object_id(db: &Database, file_id: Uuid) -> Option<Uuid> {
    sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT storage_object_id FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load file storage_object_id")
}

async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.library_files WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up library files");
    sqlx::query("DELETE FROM context69.library_folders WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up library folders");
    sqlx::query("DELETE FROM context69.library_storage_objects WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up storage objects");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

#[tokio::test]
async fn duplicate_content_creates_distinct_file_rows_sharing_storage() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping duplicate content test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let store = LibraryStore::new(db.clone());
    let group_id = seed_group(&db).await;
    let sha = "a".repeat(64);
    let storage_object_id = insert_storage_object(&db, group_id, &sha).await;

    let first = store
        .create_file_in_project(
            group_id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: Some("disclosure-A".to_string()),
                filename: "report.pdf".to_string(),
                media_type: "application/pdf".to_string(),
                size_bytes: 64,
                sha256: sha.clone(),
                storage_rel_path: format!("objects/{group_id}/{sha}"),
                storage_object_id: Some(storage_object_id),
            },
        )
        .await
        .expect("create first file");
    assert_eq!(first.external_id.as_deref(), Some("disclosure-A"));

    let second = store
        .create_file_in_project(
            group_id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: Some("disclosure-B".to_string()),
                filename: "report (2).pdf".to_string(),
                media_type: "application/pdf".to_string(),
                size_bytes: 64,
                sha256: sha.clone(),
                storage_rel_path: format!("objects/{group_id}/{sha}"),
                storage_object_id: Some(storage_object_id),
            },
        )
        .await
        .expect("create second file with same SHA and distinct external_id");

    assert_eq!(second.external_id.as_deref(), Some("disclosure-B"));
    assert_eq!(
        file_storage_object_id(&db, second.id).await,
        Some(storage_object_id)
    );
    assert_ne!(first.id, second.id);

    // The original external_id must remain intact: shared bytes never rewrite
    // the existing file's metadata.
    assert_eq!(
        file_external_id(&db, first.id).await.as_deref(),
        Some("disclosure-A")
    );
    assert_eq!(
        file_external_id(&db, second.id).await.as_deref(),
        Some("disclosure-B")
    );

    // The shared storage object must remain while either file row references it.
    assert_eq!(
        count_files_for_storage_object(&db, storage_object_id).await,
        2
    );

    let deleted = store
        .delete_file_record_in_project(group_id, first.id)
        .await
        .expect("delete first file");
    assert!(deleted, "the first duplicate-content row must be deletable");

    // Removing one file row must not orphan the still-referenced storage object.
    let still_present: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM context69.library_storage_objects WHERE id = $1")
            .bind(storage_object_id)
            .fetch_optional(db.pool())
            .await
            .expect("storage object must survive");
    assert_eq!(
        still_present,
        Some(storage_object_id),
        "storage object must survive while another library_files row references it"
    );
    assert_eq!(
        count_files_for_storage_object(&db, storage_object_id).await,
        1
    );

    // The remaining file row keeps its own external_id after sibling deletion.
    assert_eq!(
        file_external_id(&db, second.id).await.as_deref(),
        Some("disclosure-B")
    );

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn duplicate_external_id_with_different_sha_still_fails() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping duplicate external_id test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let store = LibraryStore::new(db.clone());
    let group_id = seed_group(&db).await;
    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);
    let object_a = insert_storage_object(&db, group_id, &sha_a).await;
    let object_b = insert_storage_object(&db, group_id, &sha_b).await;

    store
        .create_file_in_project(
            group_id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: Some("shared-id".to_string()),
                filename: "first.pdf".to_string(),
                media_type: "application/pdf".to_string(),
                size_bytes: 64,
                sha256: sha_a.clone(),
                storage_rel_path: format!("objects/{group_id}/{sha_a}"),
                storage_object_id: Some(object_a),
            },
        )
        .await
        .expect("create first file with shared external_id");

    // A different SHA paired with the same external_id must surface as a
    // unique-constraint violation, not be promoted to a duplicate-content
    // reuse.
    let error = store
        .create_file_in_project(
            group_id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: Some("shared-id".to_string()),
                filename: "second.pdf".to_string(),
                media_type: "application/pdf".to_string(),
                size_bytes: 64,
                sha256: sha_b.clone(),
                storage_rel_path: format!("objects/{group_id}/{sha_b}"),
                storage_object_id: Some(object_b),
            },
        )
        .await
        .expect_err("duplicate external_id with different SHA must fail");
    let message = error.to_string();
    assert!(
        message.contains("uq_library_files_group_external_id") || message.contains("duplicate key"),
        "expected unique-constraint failure, got: {message}"
    );

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn duplicate_content_filename_collision_is_resolved_per_folder() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping duplicate filename test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let store = LibraryStore::new(db.clone());
    let group_id = seed_group(&db).await;
    let sha = "c".repeat(64);
    let storage_object_id = insert_storage_object(&db, group_id, &sha).await;

    let folder_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_folders \
         (id, group_id, parent_id, name, visibility) \
         VALUES ($1, $2, NULL, $3, 'public')",
    )
    .bind(folder_id)
    .bind(group_id)
    .bind("reports")
    .execute(db.pool())
    .await
    .expect("seed test folder");

    let first = store
        .create_file_in_project(
            group_id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: Some(folder_id),
                external_id: Some("dup-content-A".to_string()),
                filename: "report.pdf".to_string(),
                media_type: "application/pdf".to_string(),
                size_bytes: 64,
                sha256: sha.clone(),
                storage_rel_path: format!("objects/{group_id}/{sha}"),
                storage_object_id: Some(storage_object_id),
            },
        )
        .await
        .expect("create first folder file");

    let occupied = store
        .list_filenames_in_project_folder(group_id, Some(folder_id), None)
        .await
        .expect("list folder filenames");
    assert!(occupied.iter().any(|name| name == "report.pdf"));

    // Mimic the production helper's suffix search so the test confirms the
    // same pattern the service relies on; the underlying
    // uq_library_files_group_folder_filename constraint would otherwise reject
    // a duplicate-content insert with the same filename.
    let base = "report.pdf";
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => (stem.to_string(), &base[stem.len()..]),
        _ => (base.to_string(), ""),
    };
    let second_filename = std::iter::successors(Some(2usize), |n| Some(n + 1))
        .map(|index| format!("{stem} ({index}){ext}"))
        .find(|candidate| !occupied.iter().any(|name| name == candidate))
        .expect("filename suffix search should terminate");
    assert_eq!(second_filename, "report (2).pdf");

    let second = store
        .create_file_in_project(
            group_id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: Some(folder_id),
                external_id: Some("dup-content-B".to_string()),
                filename: second_filename.clone(),
                media_type: "application/pdf".to_string(),
                size_bytes: 64,
                sha256: sha.clone(),
                storage_rel_path: format!("objects/{group_id}/{sha}"),
                storage_object_id: Some(storage_object_id),
            },
        )
        .await
        .expect("create second folder file with unique filename");

    assert_eq!(first.filename, "report.pdf");
    assert_eq!(second.filename, "report (2).pdf");
    assert_eq!(
        file_storage_object_id(&db, second.id).await,
        Some(storage_object_id)
    );

    // And, for completeness, confirm that reusing the original filename is
    // still rejected by the underlying constraint so we know our service code
    // really does need the suffix helper to land a duplicate.
    let dup_error = store
        .create_file_in_project(
            group_id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: Some(folder_id),
                external_id: Some("dup-content-C".to_string()),
                filename: "report.pdf".to_string(),
                media_type: "application/pdf".to_string(),
                size_bytes: 64,
                sha256: sha.clone(),
                storage_rel_path: format!("objects/{group_id}/{sha}"),
                storage_object_id: Some(storage_object_id),
            },
        )
        .await
        .expect_err("reusing the original filename must fail");
    let message = dup_error.to_string();
    assert!(
        message.contains("uq_library_files_group_folder_filename")
            || message.contains("duplicate key"),
        "expected folder-filename constraint failure, got: {message}"
    );

    cleanup_group(&db, group_id).await;
}
