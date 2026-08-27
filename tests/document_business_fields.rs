//! Phase 2 (issue 50) regression coverage for the document business-fields
//! update path. The parent document rewrite stays unconditional — extracted
//! metadata publishing may legitimately change external_id / source_uri /
//! published_at / updated_at_source / metadata_json even when the
//! `record_hash` is unchanged — but the chunk record_hash rewrite now uses a
//! null-safe `record_hash IS DISTINCT FROM $7` predicate so the chunk UPDATE
//! is a true no-op when the hash is already on disk.
//!
//! Two properties must hold:
//!
//! 1. Same-hash publish must not touch any document_chunks row (no per-row
//!    rewrite, no `updated_at` bump for unchanged hashes).
//! 2. Changed hash must propagate to every chunk of the document and bump
//!    `updated_at`.
//!
//! These tests run only when `CONTEXT69_TEST_DATABASE_URL` points at a
//! scratch database (migrations are applied automatically); they print a
//! skip message otherwise so the normal workspace build stays green without
//! the scratch DB, matching every other library_ingest_* / search_visibility
//! integration test in this repo.

use std::time::Duration;

use context69::db::Database;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use tokio::time::sleep;
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
    .bind(format!("business-fields-{}", Uuid::new_v4()))
    .bind("Business Fields Test Group")
    .bind(format!("test/business-fields-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

async fn seed_document_with_chunks(db: &Database, group_id: i64, hash: &str) -> i64 {
    let document_id = sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, \
          updated_at_source, record_hash, metadata_json, visibility) \
         VALUES ($1, 'business-fields-test', $2, 'phase 2 regression', NULL, \
          'https://example.test/phase-2', now(), $3, '{}'::jsonb, 'public') RETURNING id",
    )
    .bind(group_id)
    .bind(format!("business-fields-{}", Uuid::new_v4()))
    .bind(hash)
    .fetch_one(db.pool())
    .await
    .expect("seed test document")
    .get("id");

    for chunk_index in 0..3 {
        sqlx::query(
            "INSERT INTO context69.document_chunks \
             (id, document_id, chunk_index, chunk_text, record_hash) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(document_id)
        .bind(chunk_index)
        .bind(format!("phase 2 chunk {chunk_index}"))
        .bind(hash)
        .execute(db.pool())
        .await
        .expect("seed test chunk");
    }
    document_id
}

async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query(
        "DELETE FROM context69.document_chunks WHERE document_id IN \
         (SELECT id FROM context69.documents WHERE group_id = $1)",
    )
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("clean up chunks");
    sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up documents");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
async fn unchanged_record_hash_is_a_noop_for_document_chunks() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping phase 2 unchanged-hash test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");

    let group_id = seed_group(&db).await;
    let hash = sha256_hex(b"phase 2 unchanged hash");
    let document_id = seed_document_with_chunks(&db, group_id, &hash).await;

    // Capture the baseline `updated_at` for every chunk on this document and
    // any non-document updated_at values so the publish cannot silently bump
    // a sibling timestamp.
    let before: Vec<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT updated_at FROM context69.document_chunks WHERE document_id = $1 ORDER BY chunk_index",
    )
    .bind(document_id)
    .fetch_all(db.pool())
    .await
    .expect("load baseline chunk updated_at");
    assert_eq!(before.len(), 3, "fixture must seed three chunks");

    // Sleep just enough that a per-chunk rewrite would measurably advance
    // `updated_at`, while staying inside the test timeout.
    sleep(Duration::from_millis(50)).await;

    // Same-hash publish: the SQL `record_hash IS DISTINCT FROM $7` predicate
    // must skip every chunk row on this document. The parent documents row
    // is still rewritten (extracted metadata publishing contract).
    db.update_library_document_business_fields(
        document_id,
        &business_fields_payload(document_id, group_id, "phase 2 regression", &hash, None),
    )
    .await
    .expect("same-hash publish must succeed");

    let after: Vec<(String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT record_hash, updated_at FROM context69.document_chunks \
         WHERE document_id = $1 ORDER BY chunk_index",
    )
    .bind(document_id)
    .fetch_all(db.pool())
    .await
    .expect("load chunk state after unchanged publish");
    assert_eq!(
        after.len(),
        3,
        "same-hash publish must not delete or insert chunk rows"
    );
    for (before_ts, (after_hash, after_ts)) in before.iter().zip(after.iter()) {
        assert_eq!(
            after_hash, &hash,
            "chunk record_hash must remain unchanged on same-hash publish"
        );
        assert_eq!(
            after_ts, before_ts,
            "chunk updated_at must not advance on same-hash publish (no-op row)"
        );
    }

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn changed_record_hash_propagates_to_every_chunk() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping phase 2 changed-hash test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");

    let group_id = seed_group(&db).await;
    let baseline_hash = sha256_hex(b"phase 2 baseline hash");
    let document_id = seed_document_with_chunks(&db, group_id, &baseline_hash).await;

    let new_hash = sha256_hex(b"phase 2 changed hash");
    db.update_library_document_business_fields(
        document_id,
        &business_fields_payload(document_id, group_id, "phase 2 regression", &new_hash, None),
    )
    .await
    .expect("changed-hash publish must succeed");

    let after: Vec<String> = sqlx::query_scalar(
        "SELECT record_hash FROM context69.document_chunks \
         WHERE document_id = $1 ORDER BY chunk_index",
    )
    .bind(document_id)
    .fetch_all(db.pool())
    .await
    .expect("load chunk state after changed publish");
    assert_eq!(
        after.len(),
        3,
        "changed-hash publish must keep all chunk rows"
    );
    for after_hash in &after {
        assert_eq!(
            after_hash, &new_hash,
            "every chunk record_hash must reflect the new hash after a changed publish"
        );
    }

    let row: (String,) =
        sqlx::query_as("SELECT record_hash FROM context69.documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(db.pool())
            .await
            .expect("load document row");
    assert_eq!(
        row.0, new_hash,
        "parent document row must also reflect the new hash"
    );

    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn unchanged_hash_publish_still_rewrites_parent_document_metadata() {
    // The parent documents UPDATE remains unconditional so extracted metadata
    // publishing can legitimately change source_uri, external_id, or
    // metadata_json even when the record_hash is unchanged. This pins the
    // narrow contract: same-hash publish must skip the chunk UPDATE while
    // still rewriting the parent documents row.
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping phase 2 parent-rewrite test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");

    let group_id = seed_group(&db).await;
    let hash = sha256_hex(b"phase 2 parent same hash");
    let document_id = seed_document_with_chunks(&db, group_id, &hash).await;

    let new_source_uri = "https://example.test/phase-2/updated";
    let new_metadata = json!({"phase2_marker": true});
    db.update_library_document_business_fields(
        document_id,
        &business_fields_payload(
            document_id,
            group_id,
            "phase 2 regression",
            &hash,
            Some((new_source_uri, new_metadata)),
        ),
    )
    .await
    .expect("parent-rewrite publish must succeed");

    let row: (String, serde_json::Value) =
        sqlx::query_as("SELECT source_uri, metadata_json FROM context69.documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(db.pool())
            .await
            .expect("load parent document");
    assert_eq!(row.0, new_source_uri, "source_uri must be refreshed");
    assert_eq!(
        row.1.get("phase2_marker").and_then(|v| v.as_bool()),
        Some(true),
        "metadata_json must be refreshed"
    );

    let chunks: Vec<(String,)> =
        sqlx::query_as("SELECT record_hash FROM context69.document_chunks WHERE document_id = $1")
            .bind(document_id)
            .fetch_all(db.pool())
            .await
            .expect("load chunks after parent-rewrite publish");
    assert_eq!(chunks.len(), 3);
    for chunk in &chunks {
        assert_eq!(
            chunk.0, hash,
            "chunks must stay on the original hash when the parent rewrites metadata"
        );
    }

    cleanup_group(&db, group_id).await;
}

fn business_fields_payload(
    document_id: i64,
    group_id: i64,
    title: &str,
    record_hash: &str,
    override_metadata: Option<(&str, serde_json::Value)>,
) -> context69::domain::ChunkPayload {
    let (source_uri, metadata_json) = match override_metadata {
        Some((uri, json)) => (uri.to_string(), json),
        None => ("https://example.test/phase-2".to_string(), json!({})),
    };
    context69::domain::ChunkPayload {
        chunk_id: Uuid::nil(),
        document_id,
        group_id,
        group_key: "business-fields-test".to_string(),
        group_path: "test/business-fields".to_string(),
        visibility: context69_contracts::Visibility::Public,
        source_key: "business-fields-test".to_string(),
        external_id: format!("business-fields-{}", Uuid::new_v4()),
        title: title.to_string(),
        summary: None,
        source_uri,
        published_at: None,
        updated_at_source: chrono::Utc::now(),
        record_hash: record_hash.to_string(),
        chunk_index: 0,
        chunk_text: "phase 2 chunk".to_string(),
        metadata_json,
        content_locale: "original".to_string(),
        source_locale: None,
        translation_provider: None,
    }
}
