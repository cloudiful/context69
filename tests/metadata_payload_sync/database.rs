//! Post-checkpoint database checks for the payload-mirror outbox.
//!
//! Registered as ignored so the graded suite selects them only through the
//! authorized reusable PostgreSQL runner after the named safety checkpoint.

use context69::db::Database;
use uuid::Uuid;

async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

async fn seed_document(db: &Database) -> (i64, i64) {
    let suffix = Uuid::new_v4().simple().to_string();
    let group_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/metadata_payload_sync/insert_group.sql",
        format!("payload-sync-{suffix}"),
        format!("Payload Sync {suffix}"),
        format!("test/payload-sync-{suffix}"),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    let document_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/metadata_payload_sync/insert_document.sql",
        group_id,
        format!("payload-sync-{suffix}"),
        format!("payload-sync-hash-{suffix}"),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed document");
    (group_id, document_id)
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn claim_complete_and_fail_are_fenced_by_the_lease() {
    let db = connect_scratch().await;
    let (group_id, document_id) = seed_document(&db).await;

    // The migration owns enqueueing; this seed only exercises outbox mechanics.
    sqlx::query_file!(
        "tests/sql/metadata_payload_sync/insert_outbox_row.sql",
        document_id
    )
    .execute(db.pool())
    .await
    .expect("seed outbox row");

    let first_lease = Uuid::new_v4();
    let claimed = db
        .claim_metadata_payload_sync(first_lease, 60)
        .await
        .expect("claim job")
        .expect("one due job");
    assert_eq!(claimed.document_id, document_id);
    assert_eq!(claimed.payload_key, "metadata_index.extraction");
    assert_eq!(claimed.attempts, 0);

    // Leased rows are skipped by a concurrent claim.
    assert!(
        db.claim_metadata_payload_sync(Uuid::new_v4(), 60)
            .await
            .expect("second claim")
            .is_none()
    );
    // A stale token cannot complete or fail the row.
    assert!(
        !db.complete_metadata_payload_sync(claimed.id, Uuid::new_v4())
            .await
            .expect("stale complete")
    );
    assert!(
        !db.fail_metadata_payload_sync(claimed.id, Uuid::new_v4(), 5, "stale")
            .await
            .expect("stale fail")
    );
    // The current lease records a sanitized failure and schedules a retry.
    assert!(
        db.fail_metadata_payload_sync(claimed.id, first_lease, 5, "transient transport error")
            .await
            .expect("fail with lease")
    );
    let failure = sqlx::query_file!(
        "tests/sql/metadata_payload_sync/read_failure_state.sql",
        claimed.id
    )
    .fetch_one(db.pool())
    .await
    .expect("read failure state");
    assert_eq!(failure.attempts, 1);
    assert_eq!(
        failure.last_error.as_deref(),
        Some("transient transport error")
    );
    assert!(failure.next_due);

    // Make the row due again and complete it with the new lease.
    sqlx::query_file!("tests/sql/metadata_payload_sync/make_due.sql", claimed.id)
        .execute(db.pool())
        .await
        .expect("make due");
    let retry_lease = Uuid::new_v4();
    let retried = db
        .claim_metadata_payload_sync(retry_lease, 60)
        .await
        .expect("claim retry")
        .expect("retry is due");
    assert_eq!(retried.id, claimed.id);
    assert_eq!(retried.attempts, 1);
    assert!(
        db.complete_metadata_payload_sync(retried.id, retry_lease)
            .await
            .expect("complete")
    );
    assert!(
        db.claim_metadata_payload_sync(Uuid::new_v4(), 60)
            .await
            .expect("claim after completion")
            .is_none()
    );

    sqlx::query_file!(
        "tests/sql/metadata_payload_sync/delete_document.sql",
        document_id
    )
    .execute(db.pool())
    .await
    .ok();
    sqlx::query_file!("tests/sql/metadata_payload_sync/delete_group.sql", group_id)
        .execute(db.pool())
        .await
        .ok();
}
