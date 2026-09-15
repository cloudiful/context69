//! Issue 402 Task B1: `recursive` resource listing.
//!
//! Covers: 3-level nesting with `recursive=true` returning only subtree
//! failed files, `recursive=false` legacy behavior, cross-group isolation
//! with group isolation inside the CTE, and pagination total correctness.
//!
//! Runs only when `CONTEXT69_TEST_DATABASE_URL` points at a scratch
//! database; skipped otherwise.

use context69::db::Database;
use context69::library_store::{LibraryStore, ResourceListQuery};
use context69::contracts::{LibraryIngestStatus, LibraryResourceKind, LibraryResourceSortBy, SortDirection};
use sqlx::Row;
use uuid::Uuid;

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn connect_scratch() -> Option<(Database, LibraryStore)> {
    let url = test_database_url()?;
    let db = Database::connect(&url).await.expect("connect test database");
    let store = LibraryStore::new(db.clone());
    Some((db, store))
}

async fn seed_group(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups \
         (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("recursive-{}", Uuid::new_v4()))
    .bind("Recursive Test Group")
    .bind(format!("test/recursive-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

async fn seed_folder(
    db: &Database,
    group_id: i64,
    parent_id: Option<Uuid>,
    name: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_folders \
         (id, group_id, parent_id, name, visibility) \
         VALUES ($1, $2, $3, $4, 'public')",
    )
    .bind(id)
    .bind(group_id)
    .bind(parent_id)
    .bind(name)
    .execute(db.pool())
    .await
    .expect("seed test folder");
    id
}

async fn seed_file(
    db: &Database,
    group_id: i64,
    folder_id: Option<Uuid>,
    filename: &str,
    status: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, folder_id, filename, media_type, size_bytes, sha256, \
          storage_rel_path, ingest_status, visibility) \
         VALUES ($1, $2, $3, $4, 'text/plain', 10, 'abc', '/objects/abc', $5, 'public')",
    )
    .bind(id)
    .bind(group_id)
    .bind(folder_id)
    .bind(filename)
    .bind(status)
    .execute(db.pool())
    .await
    .expect("seed test file");
    id
}

async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.library_files WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.library_folders WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

async fn list_file_ids(
    store: &LibraryStore,
    project_id: i64,
    folder_id: Option<Uuid>,
    recursive: bool,
    status: Option<LibraryIngestStatus>,
    limit: i64,
    offset: i64,
) -> Vec<Uuid> {
    let items = store
        .list_resources_in_project_folder(&ResourceListQuery {
            project_id: Some(project_id),
            folder_id,
            recursive,
            query: None,
            status,
            sort_by: LibraryResourceSortBy::Name,
            sort_direction: SortDirection::Asc,
            limit,
            offset,
        })
        .await
        .expect("list resources");
    items
        .into_iter()
        .filter(|item| item.kind == LibraryResourceKind::File)
        .map(|item| item.id)
        .collect()
}

#[tokio::test]
async fn recursive_true_returns_only_subtree_failed_files() {
    let Some((db, store)) = connect_scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping database test");
        return;
    };
    let group_id = seed_group(&db).await;
    let level1 = seed_folder(&db, group_id, None, "level1").await;
    let level2 = seed_folder(&db, group_id, Some(level1), "level2").await;
    let level3 = seed_folder(&db, group_id, Some(level2), "level3").await;
    let sibling = seed_folder(&db, group_id, None, "sibling").await;

    let file_l1 = seed_file(&db, group_id, Some(level1), "l1-failed.txt", "failed").await;
    let file_l2 = seed_file(&db, group_id, Some(level2), "l2-failed.txt", "failed").await;
    let file_l3 = seed_file(&db, group_id, Some(level3), "l3-failed.txt", "failed").await;
    let _sibling_failed = seed_file(&db, group_id, Some(sibling), "sib-failed.txt", "failed").await;
    let _l2_succeeded = seed_file(&db, group_id, Some(level2), "l2-ok.txt", "succeeded").await;
    let _root_failed = seed_file(&db, group_id, None, "root-failed.txt", "failed").await;

    let ids = list_file_ids(
        &store,
        group_id,
        Some(level1),
        true,
        Some(LibraryIngestStatus::Failed),
        100,
        0,
    )
    .await;
    assert!(ids.contains(&file_l1), "recursive must include files directly in the root of the subtree");
    assert!(ids.contains(&file_l2), "recursive must include nested failed files");
    assert!(ids.contains(&file_l3), "recursive must include 3rd-level failed files");
    assert_eq!(ids.len(), 3, "recursive must exclude sibling, root, and non-failed files: {ids:?}");

    // The queried folder itself must not appear as a folder row in recursive mode.
    let items = store
        .list_resources_in_project_folder(&ResourceListQuery {
            project_id: Some(group_id),
            folder_id: Some(level1),
            recursive: true,
            query: None,
            status: None,
            sort_by: LibraryResourceSortBy::Name,
            sort_direction: SortDirection::Asc,
            limit: 100,
            offset: 0,
        })
        .await
        .expect("list recursive items");
    assert!(
        !items.iter().any(|item| item.kind == LibraryResourceKind::Folder && item.id == level1),
        "recursive listing must not include the queried folder itself"
    );

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn recursive_false_behavior_unchanged() {
    let Some((db, store)) = connect_scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping database test");
        return;
    };
    let group_id = seed_group(&db).await;
    let level1 = seed_folder(&db, group_id, None, "level1").await;
    let level2 = seed_folder(&db, group_id, Some(level1), "level2").await;

    let file_l1 = seed_file(&db, group_id, Some(level1), "l1-failed.txt", "failed").await;
    let _file_l2 = seed_file(&db, group_id, Some(level2), "l2-failed.txt", "failed").await;

    let ids = list_file_ids(
        &store,
        group_id,
        Some(level1),
        false,
        Some(LibraryIngestStatus::Failed),
        100,
        0,
    )
    .await;
    assert_eq!(ids, vec![file_l1], "non-recursive must return only direct children");

    let items = store
        .list_resources_in_project_folder(&ResourceListQuery {
            project_id: Some(group_id),
            folder_id: Some(level1),
            recursive: false,
            query: None,
            status: None,
            sort_by: LibraryResourceSortBy::Name,
            sort_direction: SortDirection::Asc,
            limit: 100,
            offset: 0,
        })
        .await
        .expect("list direct children");
    assert!(
        items.iter().any(|item| item.kind == LibraryResourceKind::Folder && item.id == level2),
        "non-recursive must list direct child folders"
    );

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn recursive_isolated_across_groups_and_whole_group_scope() {
    let Some((db, store)) = connect_scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping database test");
        return;
    };
    let group_a = seed_group(&db).await;
    let group_b = seed_group(&db).await;
    let folder_a = seed_folder(&db, group_a, None, "folder-a").await;
    let folder_b = seed_folder(&db, group_b, None, "folder-b").await;
    let file_a = seed_file(&db, group_a, Some(folder_a), "a-failed.txt", "failed").await;
    let file_b = seed_file(&db, group_b, Some(folder_b), "b-failed.txt", "failed").await;

    // Whole-group scope: folder_id=None + recursive=true.
    let ids_a = list_file_ids(&store, group_a, None, true, Some(LibraryIngestStatus::Failed), 100, 0).await;
    assert!(ids_a.contains(&file_a), "whole-group scope must include own files");
    assert!(!ids_a.contains(&file_b), "group isolation must hold inside the CTE");

    // Subtree query in group A must never leak group B rows even though the
    // CTE joins through parent links.
    let ids_sub = list_file_ids(&store, group_a, Some(folder_a), true, Some(LibraryIngestStatus::Failed), 100, 0).await;
    assert_eq!(ids_sub, vec![file_a], "subtree scope must stay within the group");

    cleanup_group(&db, group_a).await;
    cleanup_group(&db, group_b).await;
}

#[tokio::test]
async fn recursive_pagination_total_matches() {
    let Some((db, store)) = connect_scratch().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping database test");
        return;
    };
    let group_id = seed_group(&db).await;
    let level1 = seed_folder(&db, group_id, None, "level1").await;
    let level2 = seed_folder(&db, group_id, Some(level1), "level2").await;
    for index in 0..5 {
        seed_file(
            &db,
            group_id,
            Some(if index % 2 == 0 { level1 } else { level2 }),
            &format!("paged-{index:02}-failed.txt"),
            "failed",
        )
        .await;
    }

    let total = store
        .count_resources_in_folder(
            Some(group_id),
            Some(level1),
            None,
            Some(LibraryIngestStatus::Failed),
            true,
        )
        .await
        .expect("count recursive");
    assert_eq!(total, 5, "count must cover the whole subtree");

    let first = list_file_ids(
        &store,
        group_id,
        Some(level1),
        true,
        Some(LibraryIngestStatus::Failed),
        2,
        0,
    )
    .await;
    let second = list_file_ids(
        &store,
        group_id,
        Some(level1),
        true,
        Some(LibraryIngestStatus::Failed),
        2,
        2,
    )
    .await;
    let third = list_file_ids(
        &store,
        group_id,
        Some(level1),
        true,
        Some(LibraryIngestStatus::Failed),
        2,
        4,
    )
    .await;
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert_eq!(third.len(), 1);
    let mut all = [first, second, third].concat();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 5, "paged recursive reads must cover the total");

    cleanup_group(&db, group_id).await;
}
