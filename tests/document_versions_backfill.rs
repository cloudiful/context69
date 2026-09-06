//! Phase 4 (issue 139) focused scratch-DB tests for the controlled
//! `file_library` backfill. Database-gated: runs only when
//! `CONTEXT69_TEST_DATABASE_URL` points at a scratch database; skips
//! otherwise and never touches the normal `.env` dev database.

#[path = "support/document_versions_backfill_helpers.rs"]
mod support;

use chrono::{DateTime, Utc};
use context69::db::{
    Database, apply_file_library_backfill, check_backfill_preflight,
    preflight_file_library_backfill, resolve_apply_database_url,
};
use serde_json::{Value, json};
use support::{
    SeedDoc, cleanup_group, seed_document, seed_file_library, seed_group, seed_unrepairable,
    test_database_url, version_count,
};
use uuid::Uuid;

#[tokio::test]
async fn file_library_scope_is_exact() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping scope test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let (file_a, _, _) = seed_file_library(
        &db,
        group_id,
        "File A",
        "https://example.test/a",
        &["alpha", "beta"],
    )
    .await;
    let (file_b, _, _) = seed_file_library(
        &db,
        group_id,
        "File B",
        "https://example.test/b",
        &["gamma"],
    )
    .await;
    // Other source_key with the same violation must stay out of scope.
    let (other, _, _) = seed_document(
        &db,
        group_id,
        SeedDoc {
            source_key: "backfill-scope-other",
            title: "Other",
            summary: None,
            source_uri: "https://example.test/o",
            published_at: None,
            metadata: json!({}),
            chunk_texts: &["other"],
            record_hash_override: None,
            reverse_insert: false,
        },
    )
    .await;
    // Already-fixed file_library document must not be listed.
    let (fixed, fixed_hash, _) =
        seed_file_library(&db, group_id, "Fixed", "https://example.test/f", &["fixed"]).await;
    sqlx::query(
        "INSERT INTO context69.document_versions \
         (document_id, record_hash, title, summary, body_text, source_uri, metadata_json) \
         VALUES ($1, $2, 'Fixed', NULL, 'fixed', 'https://example.test/f', '{}'::jsonb)",
    )
    .bind(fixed)
    .bind(&fixed_hash)
    .execute(db.pool())
    .await
    .expect("seed fixed version");

    let preflight = preflight_file_library_backfill(db.pool(), 100, 1000, 10)
        .await
        .expect("preflight");
    assert_eq!(preflight.scanned, 2);
    assert_eq!(preflight.eligible, 2);
    assert!(preflight.eligible_ids.contains(&file_a));
    assert!(preflight.eligible_ids.contains(&file_b));
    assert!(!preflight.eligible_ids.contains(&other));
    assert!(!preflight.eligible_ids.contains(&fixed));
    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn preflight_count_guard_enforces_expected_scope() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping guard test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    seed_file_library(
        &db,
        group_id,
        "Guard",
        "https://example.test/g",
        &["one", "two"],
    )
    .await;
    let preflight = preflight_file_library_backfill(db.pool(), 100, 1000, 10)
        .await
        .expect("preflight");
    assert_eq!(preflight.eligible, 1);
    check_backfill_preflight(&preflight, 1).expect("exact count must pass");
    assert!(check_backfill_preflight(&preflight, 2).is_err());
    assert!(check_backfill_preflight(&preflight, 482).is_err());
    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn successful_backfill_persists_ordered_full_snapshot() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping snapshot test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let published_at: DateTime<Utc> = "2024-01-02T03:04:05Z".parse().expect("timestamp");
    let metadata = json!({"backfill_marker": "ordered"});
    let (document_id, hash, _) = seed_document(
        &db,
        group_id,
        SeedDoc {
            source_key: "file_library",
            title: "Ordered Title",
            summary: Some("Ordered summary"),
            source_uri: "https://example.test/ordered",
            published_at: Some(published_at),
            metadata: metadata.clone(),
            chunk_texts: &["alpha body", "beta body", "gamma body"],
            record_hash_override: None,
            reverse_insert: true,
        },
    )
    .await;

    let preflight = preflight_file_library_backfill(db.pool(), 100, 1000, 10)
        .await
        .expect("preflight");
    assert!(preflight.eligible_ids.contains(&document_id));
    check_backfill_preflight(&preflight, preflight.eligible).expect("guard");
    let summary = apply_file_library_backfill(db.pool(), &[document_id])
        .await
        .expect("apply");
    assert_eq!(summary.inserted, 1);
    assert_eq!(summary.inserted_ids, vec![document_id]);

    let row: (
        String,
        Option<String>,
        String,
        String,
        Option<DateTime<Utc>>,
        Value,
    ) = sqlx::query_as(
        "SELECT title, summary, body_text, source_uri, published_at, metadata_json \
         FROM context69.document_versions WHERE document_id = $1 AND record_hash = $2",
    )
    .bind(document_id)
    .bind(&hash)
    .fetch_one(db.pool())
    .await
    .expect("load backfill snapshot");
    assert_eq!(row.0, "Ordered Title");
    assert_eq!(row.1.as_deref(), Some("Ordered summary"));
    assert_eq!(row.2, "alpha body\nbeta body\ngamma body");
    assert_eq!(row.3, "https://example.test/ordered");
    assert_eq!(row.4, Some(published_at));
    assert_eq!(
        row.5.get("backfill_marker").and_then(|v| v.as_str()),
        Some("ordered")
    );
    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn hash_mismatch_is_rejected_without_write() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping hash test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let mismatch_hash = format!("mismatch-{}", Uuid::new_v4());
    let document_id = seed_unrepairable(
        &db,
        group_id,
        "Mismatch",
        "https://example.test/m",
        &["alpha", "beta"],
        &mismatch_hash,
    )
    .await;
    let preflight = preflight_file_library_backfill(db.pool(), 100, 1000, 10)
        .await
        .expect("preflight");
    assert_eq!(preflight.hash_mismatch, 1);
    assert_eq!(preflight.eligible, 0);
    let summary = apply_file_library_backfill(db.pool(), &[document_id])
        .await
        .expect("apply");
    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.skipped_docs[0].reason, "hash_mismatch");
    assert_eq!(version_count(&db, document_id).await, 0);
    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn empty_and_invalid_chunks_are_rejected() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping invalid test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let zero_hash = format!("zero-{}", Uuid::new_v4());
    let zero = seed_unrepairable(
        &db,
        group_id,
        "Zero",
        "https://example.test/z",
        &[],
        &zero_hash,
    )
    .await;
    let blank_hash = format!("blank-{}", Uuid::new_v4());
    let blank = seed_unrepairable(
        &db,
        group_id,
        "Blank",
        "https://example.test/b",
        &["   ", "\n\t "],
        &blank_hash,
    )
    .await;
    // Gapped indexes 0,2.
    let gap_hash = format!("gap-{}", Uuid::new_v4());
    let gapped = seed_unrepairable(
        &db,
        group_id,
        "Gapped",
        "https://example.test/g",
        &["first", "third"],
        &gap_hash,
    )
    .await;
    sqlx::query("UPDATE context69.document_chunks SET chunk_index = 2 WHERE document_id = $1 AND chunk_index = 1")
        .bind(gapped)
        .execute(db.pool())
        .await
        .expect("gap fixture");
    // Duplicate indexes 0,0,1.
    let dupe_hash = format!("dupe-{}", Uuid::new_v4());
    let dupe = seed_unrepairable(
        &db,
        group_id,
        "Dupe",
        "https://example.test/d",
        &["first", "second"],
        &dupe_hash,
    )
    .await;
    sqlx::query("UPDATE context69.document_chunks SET chunk_index = 0 WHERE document_id = $1 AND chunk_index = 1")
        .bind(dupe)
        .execute(db.pool())
        .await
        .expect("dupe fixture");

    let preflight = preflight_file_library_backfill(db.pool(), 100, 1000, 10)
        .await
        .expect("preflight");
    assert_eq!(preflight.zero_chunks, 1);
    assert_eq!(preflight.blank_body, 1);
    assert_eq!(preflight.non_contiguous_or_duplicate, 2);
    let summary = apply_file_library_backfill(db.pool(), &[zero, blank, gapped, dupe])
        .await
        .expect("apply");
    assert_eq!(summary.skipped, 4);
    assert_eq!(summary.inserted, 0);
    for id in [zero, blank, gapped, dupe] {
        assert_eq!(version_count(&db, id).await, 0);
    }
    cleanup_group(&db, group_id).await;
}

#[tokio::test]
async fn idempotent_rerun_does_not_overwrite() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping idempotency test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let (document_id, hash, _) = seed_file_library(
        &db,
        group_id,
        "Repeat",
        "https://example.test/r",
        &["first", "second"],
    )
    .await;
    let first = apply_file_library_backfill(db.pool(), &[document_id])
        .await
        .expect("first apply");
    assert_eq!(first.inserted, 1);
    let second = apply_file_library_backfill(db.pool(), &[document_id])
        .await
        .expect("second apply");
    assert_eq!(second.already_fixed, 1);
    assert_eq!(second.inserted, 0);
    assert_eq!(version_count(&db, document_id).await, 1);

    // A pre-existing snapshot must survive a rerun even when the stored
    // body was changed out of band: the idempotent insert never overwrites.
    sqlx::query("UPDATE context69.document_versions SET body_text = 'tampered' WHERE document_id = $1 AND record_hash = $2")
        .bind(document_id)
        .bind(&hash)
        .execute(db.pool())
        .await
        .expect("tamper version");
    let third = apply_file_library_backfill(db.pool(), &[document_id])
        .await
        .expect("third apply");
    assert_eq!(third.already_fixed, 1);
    let body: String = sqlx::query_scalar(
        "SELECT body_text FROM context69.document_versions WHERE document_id = $1 AND record_hash = $2",
    )
    .bind(document_id)
    .bind(&hash)
    .fetch_one(db.pool())
    .await
    .expect("load tampered body");
    assert_eq!(body, "tampered");
    cleanup_group(&db, group_id).await;
}

#[test]
fn apply_mode_requires_explicit_database_url_without_env_fallback() {
    let prior_app = std::env::var("CONTEXT69_APP_DB__URL").ok();
    let prior_db = std::env::var("DATABASE_URL").ok();
    unsafe {
        std::env::set_var("CONTEXT69_APP_DB__URL", "postgres://env-fallback/app");
        std::env::set_var("DATABASE_URL", "postgres://env-fallback/db");
    }
    let missing = resolve_apply_database_url(None);
    assert!(missing.is_err(), "apply without --database-url must fail");
    assert!(missing.unwrap_err().to_string().contains("--database-url"));
    let blank = resolve_apply_database_url(Some("   "));
    assert!(blank.is_err(), "blank URL must fail");
    let explicit = resolve_apply_database_url(Some("postgres://explicit/db"))
        .expect("explicit URL must succeed");
    assert_eq!(explicit, "postgres://explicit/db");
    unsafe {
        match prior_app {
            Some(value) => std::env::set_var("CONTEXT69_APP_DB__URL", value),
            None => std::env::remove_var("CONTEXT69_APP_DB__URL"),
        }
        match prior_db {
            Some(value) => std::env::set_var("DATABASE_URL", value),
            None => std::env::remove_var("DATABASE_URL"),
        }
    }
}
