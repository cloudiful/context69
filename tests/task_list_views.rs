//! Regression tests for typed task list views (issue 353, breaking B1 in
//! issue 399).
//!
//! Processing must exclude succeeded and trashed rows, completed must be
//! succeeded and non-trashed, trash must be trashed. List items and count
//! totals must agree within each view, and a user status filter may narrow
//! but never widen a view. The legacy `trashed` filter is gone: `view` is
//! required and unknown `trashed` shapes are rejected at the contract layer.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("list-view-test-{}", Uuid::new_v4()))
    .bind("List View Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn create_task(db: &Database, user_id: i64, tag: &str) -> Uuid {
    let (task_id, _, _) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/list-views"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("list-view-hash-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    task_id
}

async fn finish_task(db: &Database, task_id: Uuid, status: &str) {
    sqlx::query(
        "UPDATE context69.task_items SET status = $2, finished_at = now() WHERE task_id = $1",
    )
    .bind(task_id)
    .bind(status)
    .execute(db.pool())
    .await
    .expect("finish task items");
    db.recompute_task(task_id).await.expect("recompute task");
}

/// A library file must belong to a group, so a group row seeds the projection.
async fn seed_group(db: &Database, tag: &str) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, full_path, visibility, kind) \
         VALUES ($1, $1, $1, 'private', 'shared') RETURNING id",
    )
    .bind(format!("list-view-group-{tag}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id")
}

/// A task whose item owns a durable library file, so the row projection has a
/// file name and a title to resolve.
async fn create_named_task(db: &Database, user_id: i64, tag: &str) -> Uuid {
    let group_id = seed_group(db, tag).await;
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/list-views"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("list-view-hash-{}", Uuid::new_v4()),
        })
        .await
        .expect("create named task");
    let file_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO context69.library_files \
           (id, filename, media_type, size_bytes, sha256, storage_rel_path, \
            ingest_status, group_id, visibility, metadata_json) \
         VALUES (gen_random_uuid(), 'report.pdf', 'application/pdf', 1, $1, $2, \
           'succeeded', $3, 'private', $4::jsonb) RETURNING id",
    )
    .bind(format!("{tag}-sha"))
    .bind(format!("list-view/{tag}.pdf"))
    .bind(group_id)
    .bind(json!({ "title": "Stored file title" }))
    .fetch_one(db.pool())
    .await
    .expect("seed library file");
    sqlx::query(
        "UPDATE context69.task_items SET file_id = $2, payload = payload || $3::jsonb WHERE id = $1",
    )
    .bind(item_ids[0])
    .bind(file_id)
    .bind(json!({ "title": "Submitted title" }))
    .execute(db.pool())
    .await
    .expect("attach the library file to the item");
    task_id
}

/// The submitted title of a task's single item, used for a task whose item has
/// no library file yet.
async fn set_payload_title(db: &Database, task_id: Uuid, title: &str) {
    sqlx::query(
        "UPDATE context69.task_items SET payload = payload || $2::jsonb \
         WHERE task_id = $1 AND ordinal = 0",
    )
    .bind(task_id)
    .bind(json!({ "title": title }))
    .execute(db.pool())
    .await
    .expect("set the item title");
}

async fn list_ids(db: &Database, user_id: i64, view: &str, status: Option<&str>) -> Vec<Uuid> {
    db.list_tasks(context69::db::TaskListFilter {
        user_id,
        query: None,
        kind: None,
        status,
        stage: None,
        waiting_reason: None,
        dependency_key: None,
        sort_by: None,
        sort_direction: None,
        limit: 50,
        offset: 0,
        view,
    })
    .await
    .expect("list tasks")
    .into_iter()
    .map(|task| task.id)
    .collect()
}

async fn count(db: &Database, user_id: i64, view: &str, status: Option<&str>) -> i64 {
    db.count_tasks(context69::db::TaskCountFilter {
        user_id,
        query: None,
        kind: None,
        status,
        stage: None,
        waiting_reason: None,
        dependency_key: None,
        view,
    })
    .await
    .expect("count tasks")
}

#[tokio::test]
async fn list_views_are_mutually_exclusive_and_counts_match() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping list view test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();

    let queued = create_task(&db, user_id, &format!("{tag}-queued")).await;
    let succeeded = create_task(&db, user_id, &format!("{tag}-succeeded")).await;
    finish_task(&db, succeeded, "succeeded").await;
    let failed = create_task(&db, user_id, &format!("{tag}-failed")).await;
    finish_task(&db, failed, "failed").await;
    let trashed_succeeded = create_task(&db, user_id, &format!("{tag}-trashed-succeeded")).await;
    finish_task(&db, trashed_succeeded, "succeeded").await;
    assert!(
        db.trash_task(trashed_succeeded)
            .await
            .expect("trash succeeded"),
        "terminal succeeded task must be trashable"
    );
    let trashed_failed = create_task(&db, user_id, &format!("{tag}-trashed-failed")).await;
    finish_task(&db, trashed_failed, "failed").await;
    assert!(
        db.trash_task(trashed_failed).await.expect("trash failed"),
        "terminal failed task must be trashable"
    );

    // Processing excludes succeeded and trashed.
    let processing = list_ids(&db, user_id, "processing", None).await;
    assert!(
        processing.contains(&queued),
        "processing must contain queued"
    );
    assert!(
        processing.contains(&failed),
        "processing must contain failed"
    );
    assert!(
        !processing.contains(&succeeded),
        "processing must exclude succeeded"
    );
    assert!(
        !processing.contains(&trashed_succeeded),
        "processing must exclude trashed"
    );
    assert!(
        !processing.contains(&trashed_failed),
        "processing must exclude trashed"
    );
    assert_eq!(
        count(&db, user_id, "processing", None).await,
        processing.len() as i64,
        "processing count must match list length"
    );

    // Completed is succeeded and non-trashed only.
    let completed = list_ids(&db, user_id, "completed", None).await;
    assert!(
        completed.contains(&succeeded),
        "completed must contain succeeded"
    );
    assert!(
        !completed.contains(&queued),
        "completed must exclude queued"
    );
    assert!(
        !completed.contains(&failed),
        "completed must exclude failed"
    );
    assert!(
        !completed.contains(&trashed_succeeded),
        "completed must exclude trashed"
    );
    assert_eq!(
        count(&db, user_id, "completed", None).await,
        completed.len() as i64,
        "completed count must match list length"
    );

    // Trash is trashed only.
    let trash = list_ids(&db, user_id, "trash", None).await;
    assert!(
        trash.contains(&trashed_succeeded),
        "trash must contain trashed succeeded"
    );
    assert!(
        trash.contains(&trashed_failed),
        "trash must contain trashed failed"
    );
    assert!(!trash.contains(&queued), "trash must exclude active");
    assert!(
        !trash.contains(&succeeded),
        "trash must exclude active succeeded"
    );
    assert!(!trash.contains(&failed), "trash must exclude active failed");
    assert_eq!(
        count(&db, user_id, "trash", None).await,
        trash.len() as i64,
        "trash count must match list length"
    );

    // Views are mutually exclusive for these fixtures.
    for id in [
        &queued,
        &succeeded,
        &failed,
        &trashed_succeeded,
        &trashed_failed,
    ] {
        let hits = [&processing, &completed, &trash]
            .iter()
            .filter(|view| view.contains(id))
            .count();
        assert_eq!(hits, 1, "task {id} must appear in exactly one view");
    }
}

#[tokio::test]
async fn a_row_projects_the_focus_item_file_name_and_document_title() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping list view projection test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();

    // One task with a durable library file and a submitted title, one with
    // only a submitted title, and one with neither.
    let named = create_named_task(&db, user_id, &format!("{tag}-named")).await;
    let titled_only = create_task(&db, user_id, &format!("{tag}-titled")).await;
    set_payload_title(&db, titled_only, "Titled but not ingested").await;
    let bare = create_task(&db, user_id, &format!("{tag}-bare")).await;

    let rows = db
        .list_tasks(context69::db::TaskListFilter {
            user_id,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "processing",
        })
        .await
        .expect("list tasks");
    let named_row = rows
        .iter()
        .find(|task| task.id == named)
        .expect("named row");
    assert_eq!(
        named_row.file_name.as_deref(),
        Some("report.pdf"),
        "a collapsed row carries the file name of its focus item"
    );
    assert_eq!(
        named_row.document_title.as_deref(),
        Some("Submitted title"),
        "the item's own title wins over the file's stored title"
    );
    let titled_row = rows
        .iter()
        .find(|task| task.id == titled_only)
        .expect("titled row");
    assert_eq!(
        titled_row.file_name, None,
        "an item with no library file projects no file name"
    );
    assert_eq!(
        titled_row.document_title.as_deref(),
        Some("Titled but not ingested"),
        "a submitted title is enough when the item has no file"
    );
    let bare_row = rows.iter().find(|task| task.id == bare).expect("bare row");
    assert_eq!(bare_row.file_name, None);
    assert_eq!(
        bare_row.document_title, None,
        "an item with neither source projects no title"
    );

    // The same projection reaches the item read, and the internal read agrees.
    let items = db
        .list_task_items(named, 10, 0)
        .await
        .expect("list task items");
    assert_eq!(items.len(), 1, "the file join must not multiply an item");
    assert_eq!(items[0].file_name.as_deref(), Some("report.pdf"));
    assert_eq!(items[0].document_title.as_deref(), Some("Submitted title"));
    let internal = db
        .get_task_internal(named)
        .await
        .expect("load task internally")
        .expect("task exists");
    assert_eq!(internal.file_name.as_deref(), Some("report.pdf"));
    assert_eq!(internal.document_title.as_deref(), Some("Submitted title"));
}

/// A cancelled item that never materialized a library file is still named by
/// what its payload retained, and an ingested item prefers the authoritative
/// title of the document it produced over the file's stored metadata title.
#[tokio::test]
async fn the_projection_falls_back_to_the_payload_and_the_linked_document() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping projection fallback test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();

    // A cancelled URL item with no library file: only the payload names it, and
    // the query/fragment must not leak into the projected file name.
    let pending = create_url_task(
        &db,
        user_id,
        &format!("{tag}-pending"),
        "https://static.example.com/reports/2026-09-30-summary.PDF?token=abc#page2",
    )
    .await;
    finish_task(&db, pending, "cancelled").await;
    let pending_row = db
        .get_task_internal(pending)
        .await
        .expect("load pending task")
        .expect("pending task exists");
    assert_eq!(
        pending_row.file_name.as_deref(),
        Some("2026-09-30-summary.PDF"),
        "an unmaterialized item is named by the last segment of its retained URL"
    );
    assert_eq!(
        pending_row.document_title, None,
        "an item with neither a submitted title nor a document projects no title"
    );

    // An ingested item whose linked document carries the authoritative title,
    // while its library file still holds a different stored metadata title.
    let ingested = create_named_task(&db, user_id, &format!("{tag}-ingested")).await;
    let group_id = file_group_of(&db, ingested).await;
    // Drop the submitted title so the only remaining title sources are the
    // linked document and the file's metadata.
    drop_submitted_title(&db, ingested).await;
    link_document(
        &db,
        group_id,
        ingested,
        "Stored file title",
        "Authoritative document title",
    )
    .await;
    let ingested_row = db
        .get_task_internal(ingested)
        .await
        .expect("load ingested task")
        .expect("ingested task exists");
    assert_eq!(ingested_row.file_name.as_deref(), Some("report.pdf"));
    assert_eq!(
        ingested_row.document_title.as_deref(),
        Some("Authoritative document title"),
        "the linked document title outranks the file's stored metadata title"
    );

    // A second section of the same file must not fan the row out, and the
    // deterministic pick stays the first section in document order.
    link_second_section(&db, ingested, "Later section title").await;
    let rows = db
        .list_task_items(ingested, 10, 0)
        .await
        .expect("list task items");
    assert_eq!(
        rows.len(),
        1,
        "several linked sections must still project one item row"
    );
    assert_eq!(
        rows[0].document_title.as_deref(),
        Some("Authoritative document title"),
        "the first section in document order wins deterministically"
    );

    // A cross-group link must never surface the other group's title, even when it
    // sorts first, and the guard must keep the row count at one.
    let foreign_group = seed_group(&db, &format!("{tag}-foreign")).await;
    link_cross_group_document(&db, ingested, foreign_group, "Foreign group title").await;
    let cross_group = db
        .list_task_items(ingested, 10, 0)
        .await
        .expect("list task items after the cross-group link");
    assert_eq!(
        cross_group.len(),
        1,
        "a cross-group link must still project one item row"
    );
    assert_eq!(
        cross_group[0].document_title.as_deref(),
        Some("Authoritative document title"),
        "the cross-group document sorts first but must never name the item"
    );
    let cross_group_task = db
        .get_task_internal(ingested)
        .await
        .expect("reload task")
        .expect("task exists");
    assert_eq!(
        cross_group_task.document_title.as_deref(),
        Some("Authoritative document title"),
        "the collapsed row must not leak a cross-group title either"
    );

    // An empty candidate falls through to the next source instead of masking it,
    // so no empty string is ever projected.
    sqlx::query(
        "UPDATE context69.documents SET title = '' \
         WHERE title = 'Authoritative document title'",
    )
    .execute(db.pool())
    .await
    .expect("blank the linked document title");
    let after_blank_document = db
        .get_task_internal(ingested)
        .await
        .expect("reload task")
        .expect("task exists");
    assert_eq!(
        after_blank_document.document_title.as_deref(),
        Some("Stored file title"),
        "an empty document title must fall through to the file's own title"
    );
    set_file_metadata_title(&db, ingested, "").await;
    let empty_candidates = db
        .get_task_internal(ingested)
        .await
        .expect("reload task")
        .expect("task exists");
    assert_eq!(
        empty_candidates.document_title, None,
        "with every candidate empty the projection is NULL, never ''"
    );
}

/// Drop the title the submission carried, leaving the document and file
/// metadata as the only title sources.
async fn drop_submitted_title(db: &Database, task_id: Uuid) {
    sqlx::query("UPDATE context69.task_items SET payload = payload - 'title' WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("drop the submitted title");
}

/// The group of the library file attached to a task's first item.
async fn file_group_of(db: &Database, task_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT file.group_id FROM context69.task_items item \
         JOIN context69.library_files file ON file.id = item.file_id \
         WHERE item.task_id = $1 AND item.ordinal = 0",
    )
    .bind(task_id)
    .fetch_one(db.pool())
    .await
    .expect("read the item file group")
}

/// Give the task's item the same external id as a document in its own group.
/// Seed the document the item's own file produced, wired through
/// `library_file_documents`, which is the authoritative link.
async fn link_document(
    db: &Database,
    group_id: i64,
    task_id: Uuid,
    file_metadata_title: &str,
    document_title: &str,
) {
    set_file_metadata_title(db, task_id, file_metadata_title).await;
    let document_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO context69.documents \
           (source_key, external_id, title, source_uri, updated_at_source, record_hash, \
            group_id, visibility) \
         VALUES ('list_view', $1, $2, 'https://static.example.com/doc', now(), $3, $4, 'private') \
         RETURNING id",
    )
    .bind(format!("list-view-{task_id}"))
    .bind(document_title)
    .bind(format!("{task_id}-doc-hash"))
    .bind(group_id)
    .fetch_one(db.pool())
    .await
    .expect("seed the linked document");
    sqlx::query(
        "INSERT INTO context69.library_file_documents \
           (file_id, document_id, section_key, section_label, sort_order, group_id, visibility) \
         SELECT file.id, $2, 'section-0', 'Section 0', 0, $3, 'private' \
         FROM context69.library_files file \
         WHERE file.id = (SELECT item.file_id FROM context69.task_items item \
                          WHERE item.task_id = $1 AND item.ordinal = 0)",
    )
    .bind(task_id)
    .bind(document_id)
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("link the document to the item file");
}

/// A second section of the same file: the projection must still return one row
/// and one title.
async fn link_second_section(db: &Database, task_id: Uuid, title: &str) {
    let document_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO context69.documents \
           (source_key, external_id, title, source_uri, updated_at_source, record_hash, \
            group_id, visibility) \
         SELECT 'list_view', $2, $3, 'https://static.example.com/doc2', now(), $4, \
            file.group_id, 'private' \
         FROM context69.library_files file \
         WHERE file.id = (SELECT item.file_id FROM context69.task_items item \
                          WHERE item.task_id = $1 AND item.ordinal = 0) \
         RETURNING id",
    )
    .bind(task_id)
    .bind(format!("list-view-{task_id}-section-2"))
    .bind(title)
    .bind(format!("{task_id}-doc2-hash"))
    .fetch_one(db.pool())
    .await
    .expect("seed the second document");
    sqlx::query(
        "INSERT INTO context69.library_file_documents \
           (file_id, document_id, section_key, section_label, sort_order, group_id, visibility) \
         SELECT file.id, $2, 'section-2', 'Section 2', 2, file.group_id, 'private' \
         FROM context69.library_files file \
         WHERE file.id = (SELECT item.file_id FROM context69.task_items item \
                          WHERE item.task_id = $1 AND item.ordinal = 0)",
    )
    .bind(task_id)
    .bind(document_id)
    .execute(db.pool())
    .await
    .expect("link the second section");
}

/// Link a document of a **different** group to the task's own file, as a
/// corrupted or migrated row would look. The projection must ignore it.
async fn link_cross_group_document(db: &Database, task_id: Uuid, foreign_group: i64, title: &str) {
    let foreign_document = sqlx::query_scalar::<_, i64>(
        "INSERT INTO context69.documents \
           (source_key, external_id, title, source_uri, updated_at_source, record_hash, \
            group_id, visibility) \
         VALUES ('list_view', $1, $2, 'https://static.example.com/foreign', now(), $3, \
            $4, 'private') RETURNING id",
    )
    .bind(format!("list-view-{task_id}-foreign"))
    .bind(title)
    .bind(format!("{task_id}-foreign-hash"))
    .bind(foreign_group)
    .fetch_one(db.pool())
    .await
    .expect("seed the foreign document");
    sqlx::query(
        "INSERT INTO context69.library_file_documents \
           (file_id, document_id, section_key, section_label, sort_order, group_id, visibility) \
         SELECT file.id, $2, 'section-foreign', 'Foreign', -1, file.group_id, 'private' \
         FROM context69.library_files file \
         WHERE file.id = (SELECT item.file_id FROM context69.task_items item \
                          WHERE item.task_id = $1 AND item.ordinal = 0)",
    )
    .bind(task_id)
    .bind(foreign_document)
    .execute(db.pool())
    .await
    .expect("link the foreign document");
}

/// The `metadata_json.title` stored on the item's own library file.
async fn set_file_metadata_title(db: &Database, task_id: Uuid, title: &str) {
    sqlx::query(
        "UPDATE context69.library_files SET metadata_json = jsonb_set( \
           metadata_json, '{title}', to_jsonb($2::text)) \
         WHERE id = (SELECT file_id FROM context69.task_items WHERE task_id = $1 AND ordinal = 0)",
    )
    .bind(task_id)
    .bind(title)
    .execute(db.pool())
    .await
    .expect("set the file metadata title");
}

/// A `url_batch` task whose single item carries only the submitted URL, i.e. no
/// library file and no title.
async fn create_url_task(db: &Database, user_id: i64, tag: &str, url: &str) -> Uuid {
    let (task_id, _, _) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "url_batch",
            group_path: Some("test/list-views"),
            source_key: None,
            payloads: &[json!({ "url": url })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("list-view-hash-{}", Uuid::new_v4()),
        })
        .await
        .expect("create url task");
    task_id
}

#[tokio::test]
async fn status_filter_narrows_but_never_widens_a_view() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping list view narrowing test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();

    let succeeded = create_task(&db, user_id, &format!("{tag}-succeeded")).await;
    finish_task(&db, succeeded, "succeeded").await;
    let failed = create_task(&db, user_id, &format!("{tag}-failed")).await;
    finish_task(&db, failed, "failed").await;

    // Processing plus succeeded widens nothing: it matches nothing.
    let processing_succeeded = list_ids(&db, user_id, "processing", Some("succeeded")).await;
    assert!(
        !processing_succeeded.contains(&succeeded),
        "processing plus status=succeeded must not widen to succeeded"
    );
    assert_eq!(
        count(&db, user_id, "processing", Some("succeeded")).await,
        processing_succeeded.len() as i64,
    );

    // Processing plus failed narrows to failed only.
    let processing_failed = list_ids(&db, user_id, "processing", Some("failed")).await;
    assert!(processing_failed.contains(&failed));
    assert!(!processing_failed.contains(&succeeded));
    assert_eq!(
        count(&db, user_id, "processing", Some("failed")).await,
        processing_failed.len() as i64,
    );

    // Completed plus failed matches nothing.
    let completed_failed = list_ids(&db, user_id, "completed", Some("failed")).await;
    assert!(
        !completed_failed.contains(&failed),
        "completed plus status=failed must not widen to failed"
    );
    assert_eq!(
        count(&db, user_id, "completed", Some("failed")).await,
        completed_failed.len() as i64,
    );
}
