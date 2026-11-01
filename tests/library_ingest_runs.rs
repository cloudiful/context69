use context69::db::{
    CreateTaskSubmissionRequest, Database, IngestRunMode, IngestRunSource, NewIngestRun,
};
use context69::library_store::{LibraryStore, NewLibraryFile, TaskWriteFence, begin_task_write};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn fenced_identity_adoption_resolution_and_rollback() {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .unwrap();
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .unwrap();
    let user = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("run-{}", Uuid::new_v4()),
        "Run Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    let group = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("run-{}", Uuid::new_v4()),
        "Run Test",
        format!("test/run-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_group_membership.sql",
        group,
        user
    )
    .execute(db.pool())
    .await
    .unwrap();
    let task = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: task,
            user_id: user,
            group_id: Some(group),
            kind: "text_batch",
            group_path: Some("test/run"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("run-{}", Uuid::new_v4()),
        })
        .await
        .unwrap();
    let item = db
        .claim_items_fast(50)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.id == items[0])
        .unwrap();
    let fence = TaskWriteFence::from_claimed_item(&item).unwrap();
    let store = LibraryStore::new(db.clone());
    let file = NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: None,
        filename: "run.txt".into(),
        media_type: "text/plain".into(),
        size_bytes: 4,
        sha256: "0".repeat(64),
        storage_rel_path: format!("objects/test/{}", Uuid::new_v4()),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    assert!(
        store
            .create_file_in_project_fenced(fence, group, &file)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.set_task_item_file(task, item.id, &item.execution_context().unwrap(), file.id)
            .await
            .unwrap()
    );
    let plan = || NewIngestRun {
        mode: IngestRunMode::FirstInsert,
        source: IngestRunSource::Stored,
    };
    let mut tx = begin_task_write(&db, fence).await.unwrap();
    let discarded = db
        .claim_or_create_library_ingest_run_in(&mut tx, task, file.id, Some(plan()))
        .await
        .unwrap();
    tx.rollback().await;
    assert!(db.read_library_ingest_run(item.id).await.unwrap().is_none());
    let create = || async {
        let mut tx = begin_task_write(&db, fence).await.unwrap();
        let run = db
            .claim_or_create_library_ingest_run_in(&mut tx, task, file.id, Some(plan()))
            .await
            .unwrap();
        assert!(tx.commit().await.unwrap());
        run
    };
    let (run, concurrent) = tokio::join!(create(), create());
    assert_eq!(run.batch_id, concurrent.batch_id);
    assert_eq!(run.generation, concurrent.generation);
    assert!(run.generation > discarded.generation);
    let execution = item.execution_context().unwrap();
    sqlx::query_file!(
        "tests/sql/task_execution/expire_library_attempt_by_id.sql",
        execution.attempt_id()
    )
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item.id)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(begin_task_write(&db, fence).await.is_err());
    let next = db
        .claim_items_fast(50)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.id == item.id)
        .unwrap();
    let fence = TaskWriteFence::from_claimed_item(&next).unwrap();
    let mut adopt = begin_task_write(&db, fence).await.unwrap();
    let adopted = db
        .claim_or_create_library_ingest_run_in(&mut adopt, task, file.id, Some(plan()))
        .await
        .unwrap();
    assert_eq!(adopted.batch_id, run.batch_id);
    assert_eq!(adopted.generation, run.generation);
    assert_eq!(adopted.lease_token, fence.token());
    assert!(adopt.commit().await.unwrap());
    let mut tx = begin_task_write(&db, fence).await.unwrap();
    assert!(
        db.claim_or_create_library_ingest_run_in(&mut tx, task, Uuid::new_v4(), Some(plan()))
            .await
            .is_err()
    );
    assert!(
        db.claim_or_create_library_ingest_run_in(
            &mut tx,
            task,
            file.id,
            Some(NewIngestRun {
                mode: IngestRunMode::Replacement,
                source: IngestRunSource::Stored
            })
        )
        .await
        .is_err()
    );
    assert!(
        db.claim_or_create_library_ingest_run_in(&mut tx, Uuid::new_v4(), file.id, Some(plan()))
            .await
            .is_err()
    );
    assert!(
        db.claim_or_create_library_ingest_run_in(
            &mut tx,
            task,
            file.id,
            Some(NewIngestRun {
                mode: IngestRunMode::FirstInsert,
                source: IngestRunSource::Url
            })
        )
        .await
        .is_err()
    );
    let retry = db
        .claim_or_create_library_ingest_run_in(&mut tx, task, file.id, Some(plan()))
        .await
        .unwrap();
    assert_eq!(retry.batch_id, run.batch_id);
    assert_eq!(retry.generation, run.generation);
    assert!(
        db.mark_library_ingest_run_resolved_in(&mut tx, task, file.id)
            .await
            .unwrap()
    );
    tx.rollback().await;
    assert!(
        db.read_library_ingest_run(item.id)
            .await
            .unwrap()
            .unwrap()
            .resolved_at
            .is_none()
    );
    // The durable section identity of a current run is reserved once and
    // reused, while another identity under the same section key is refused.
    let reserved = db
        .reserve_library_ingest_run_section(fence, &adopted, "section-a", "external-a")
        .await
        .unwrap();
    let reused = db
        .reserve_library_ingest_run_section(fence, &adopted, "section-a", "external-a")
        .await
        .unwrap();
    assert_eq!(reserved, reused);
    assert!(
        db.reserve_library_ingest_run_section(fence, &adopted, "section-a", "external-b")
            .await
            .is_err()
    );
    let mut tx = begin_task_write(&db, fence).await.unwrap();
    assert!(
        db.mark_library_ingest_run_resolved_in(&mut tx, task, file.id)
            .await
            .unwrap()
    );
    assert!(
        !db.mark_library_ingest_run_resolved_in(&mut tx, task, file.id)
            .await
            .unwrap()
    );
    assert!(tx.commit().await.unwrap());
    // A resolved run is no longer current, so it cannot reserve a new section.
    assert!(
        db.reserve_library_ingest_run_section(fence, &adopted, "section-b", "external-b")
            .await
            .is_err()
    );
    let mut tx = begin_task_write(&db, fence).await.unwrap();
    assert!(
        db.claim_or_create_library_ingest_run_in(&mut tx, task, file.id, Some(plan()))
            .await
            .unwrap()
            .resolved_at
            .is_some()
    );
    assert!(tx.commit().await.unwrap());
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query_file!("tests/sql/task_execution/delete_group_files.sql", group)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user)
        .execute(db.pool())
        .await
        .unwrap();
}
