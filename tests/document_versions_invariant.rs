//! Phase 2 (issue 139) regression for the library version invariant.
//!
//! `update_library_document_business_fields` must persist a complete
//! `document_versions` snapshot when the business/metadata hash changes,
//! stay idempotent on same-hash publishes, fail closed (rollback) when no
//! reconstructable body exists, and never overwrite an existing snapshot on
//! repeated or concurrent invocation.
//!
//! Database-gated: runs only when `CONTEXT69_TEST_DATABASE_URL` points at a
//! scratch database; skips otherwise so the normal build stays green.

#[path = "support/document_versions_invariant.rs"]
mod support;

use context69::db::Database;
use context69_extraction::ExtractionStore;
use serde_json::json;
use support::{
    business_payload, cleanup_group, document_hash, seed_document_with_chunks, seed_group,
    test_database_url, version_body, version_count,
};
use uuid::Uuid;

#[tokio::test]
async fn changed_hash_creates_complete_ordered_snapshot() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping changed-hash snapshot test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let baseline = format!("baseline-{}", Uuid::new_v4());
    let document_id = seed_document_with_chunks(
        &db,
        group_id,
        &baseline,
        "invariant title",
        &["alpha body", "beta body", "gamma body"],
    )
    .await;

    let new_hash = format!("changed-{}", Uuid::new_v4());
    let metadata = json!({"invariant_marker": true});
    db.update_library_document_business_fields(
        document_id,
        &business_payload(document_id, group_id, &new_hash, metadata.clone()),
    )
    .await
    .expect("changed-hash publish must succeed");

    assert_eq!(document_hash(&db, document_id).await, new_hash);
    let body = version_body(&db, document_id, &new_hash)
        .await
        .expect("changed hash must persist a matching version row");
    assert_eq!(body, "alpha body\nbeta body\ngamma body");
    let row: (String, String, serde_json::Value) = sqlx::query_as(
        "SELECT title, source_uri, metadata_json FROM context69.document_versions \
         WHERE document_id = $1 AND record_hash = $2",
    )
    .bind(document_id)
    .bind(&new_hash)
    .fetch_one(db.pool())
    .await
    .expect("load version snapshot fields");
    assert_eq!(row.0, "invariant title");
    assert_eq!(row.1, "https://example.test/version-invariant/updated");
    assert_eq!(
        row.2.get("invariant_marker").and_then(|v| v.as_bool()),
        Some(true)
    );

    // Extraction read path joins the current hash; it must succeed now.
    let store = ExtractionStore::new(db.pool().clone());
    let doc = store
        .document(document_id)
        .await
        .expect("extraction load must succeed");
    assert_eq!(doc.record_hash, new_hash);
    assert_eq!(doc.body_text, "alpha body\nbeta body\ngamma body");

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn same_hash_publish_is_idempotent_without_duplicate_versions() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping same-hash idempotency test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let hash = format!("same-{}", Uuid::new_v4());
    let document_id =
        seed_document_with_chunks(&db, group_id, &hash, "invariant title", &["one", "two"]).await;

    db.update_library_document_business_fields(
        document_id,
        &business_payload(document_id, group_id, &hash, json!({})),
    )
    .await
    .expect("same-hash publish must succeed");
    assert_eq!(version_count(&db, document_id).await, 0);

    // Seed the matching version once, then repeat the same-hash publish: the
    // second call must not duplicate or touch the row.
    sqlx::query(
        "INSERT INTO context69.document_versions \
         (document_id, record_hash, title, summary, body_text, source_uri, metadata_json) \
         VALUES ($1, $2, 'invariant title', 'invariant summary', 'one\ntwo', \
          'https://example.test/version-invariant/updated', '{}'::jsonb)",
    )
    .bind(document_id)
    .bind(&hash)
    .execute(db.pool())
    .await
    .expect("seed baseline version");
    db.update_library_document_business_fields(
        document_id,
        &business_payload(document_id, group_id, &hash, json!({})),
    )
    .await
    .expect("repeated same-hash publish must succeed");
    assert_eq!(version_count(&db, document_id).await, 1);
    assert_eq!(
        version_body(&db, document_id, &hash).await.as_deref(),
        Some("one\ntwo")
    );

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn changed_hash_with_no_chunks_fails_and_rolls_back() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping empty-chunks rollback test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let baseline = format!("empty-baseline-{}", Uuid::new_v4());
    let document_id =
        seed_document_with_chunks(&db, group_id, &baseline, "invariant title", &[]).await;

    let new_hash = format!("empty-changed-{}", Uuid::new_v4());
    let err = db
        .update_library_document_business_fields(
            document_id,
            &business_payload(document_id, group_id, &new_hash, json!({})),
        )
        .await
        .expect_err("changed hash with no chunks must fail");
    assert!(
        err.to_string().contains("no chunks"),
        "unexpected error: {err:#}"
    );

    assert_eq!(document_hash(&db, document_id).await, baseline);
    assert_eq!(version_body(&db, document_id, &new_hash).await, None);

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn changed_hash_with_blank_body_fails_and_rolls_back() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping blank-body rollback test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let baseline = format!("blank-baseline-{}", Uuid::new_v4());
    let document_id = seed_document_with_chunks(
        &db,
        group_id,
        &baseline,
        "invariant title",
        &["   ", "\n\t "],
    )
    .await;

    let new_hash = format!("blank-changed-{}", Uuid::new_v4());
    let err = db
        .update_library_document_business_fields(
            document_id,
            &business_payload(document_id, group_id, &new_hash, json!({})),
        )
        .await
        .expect_err("changed hash with blank body must fail");
    assert!(
        err.to_string().contains("no valid body text"),
        "unexpected error: {err:#}"
    );

    assert_eq!(document_hash(&db, document_id).await, baseline);
    assert_eq!(version_body(&db, document_id, &new_hash).await, None);

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn repeated_invocation_does_not_overwrite_existing_snapshot() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping repeated-invocation test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let baseline = format!("repeat-baseline-{}", Uuid::new_v4());
    let document_id = seed_document_with_chunks(
        &db,
        group_id,
        &baseline,
        "invariant title",
        &["first", "second"],
    )
    .await;

    let new_hash = format!("repeat-changed-{}", Uuid::new_v4());
    let payload = business_payload(document_id, group_id, &new_hash, json!({"n": 1}));
    let second = Database::clone(&db);
    let payload_clone = payload.clone();
    let (first_result, second_result) = tokio::join!(
        db.update_library_document_business_fields(document_id, &payload),
        second.update_library_document_business_fields(document_id, &payload_clone),
    );
    first_result.expect("concurrent publish must succeed");
    second_result.expect("concurrent publish must succeed");
    assert_eq!(version_count(&db, document_id).await, 1);
    assert_eq!(
        version_body(&db, document_id, &new_hash).await.as_deref(),
        Some("first\nsecond")
    );

    // A repeated call after the hash already matches is a no-op and must keep
    // the original snapshot body even though the payload only carries the
    // first chunk.
    db.update_library_document_business_fields(
        document_id,
        &business_payload(document_id, group_id, &new_hash, json!({"n": 2})),
    )
    .await
    .expect("repeated publish must succeed");
    assert_eq!(version_count(&db, document_id).await, 1);
    assert_eq!(
        version_body(&db, document_id, &new_hash).await.as_deref(),
        Some("first\nsecond")
    );

    cleanup_group(&db, group_id).await;
}
