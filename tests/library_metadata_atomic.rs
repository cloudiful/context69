#![cfg(feature = "integration-test-helpers")]

//! Atomic metadata publication regressions (issue 1138, P19).
//!
//! These run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and compiles the workspace tests with the
//! `integration-test-helpers` feature. A plain `cargo test --workspace` skips
//! them as ignored, and they are reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.

#[path = "library_metadata_atomic/support.rs"]
mod support;

use context69::contracts::{CanonicalUploadMetadata, MoveFileRequest};
use context69::domain::{ChunkPayload, SourceRecord};
use context69::normalize::normalize_record;
use context69_contracts::Visibility;
use sqlx::Row;
use support::{CANONICAL_BODY, MetadataFixture};
use uuid::Uuid;

/// The explicit search-registry admission caps of the metadata publisher.
const CAPS: [i64; 3] = [1000, 1000, 1000];

fn metadata(tag: &str) -> CanonicalUploadMetadata {
    let mut map = std::collections::BTreeMap::new();
    map.insert("tag".to_string(), serde_json::json!(tag));
    CanonicalUploadMetadata {
        external_id: None,
        source_uri: None,
        published_at: None,
        metadata_json: map,
    }
}

async fn source_fields(
    fixture: &MetadataFixture,
    document_id: i64,
) -> (String, Option<String>, String) {
    let row =
        sqlx::query("SELECT title, summary, source_uri FROM context69.documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read document fields");
    (row.get(0), row.get(1), row.get(2))
}

async fn stored_chunks(fixture: &MetadataFixture, document_id: i64) -> Vec<(Uuid, String)> {
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, chunk_text FROM context69.document_chunks \
         WHERE document_id = $1 ORDER BY chunk_index",
    )
    .bind(document_id)
    .fetch_all(fixture.db.pool())
    .await
    .expect("read stored chunks")
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_canonical_version_body_is_the_hash_basis_not_overlapping_chunks() {
    let fixture = MetadataFixture::new().await;
    let document = fixture
        .seed_document(
            "section-one",
            0,
            CANONICAL_BODY,
            &["alpha beta gamma", "gamma delta epsilon"],
        )
        .await;

    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("publish metadata");

    let record_hash = fixture.document_hash(document.document_id).await;
    let body = fixture
        .version_body(document.document_id, &record_hash)
        .await
        .expect("the new hash has a version body");
    let stored_metadata = fixture.document_metadata(document.document_id).await;
    let (title, summary, source_uri) = source_fields(&fixture, document.document_id).await;
    let expected = normalize_record(SourceRecord {
        external_id: "not-hashed".into(),
        title,
        summary,
        body_text: body.clone(),
        source_uri,
        published_at: None,
        updated_at: chrono::Utc::now(),
        metadata_json: stored_metadata.clone(),
    })
    .record_hash;
    assert_eq!(
        record_hash, expected,
        "the stored hash must cover the canonical version body"
    );

    let joined = stored_chunks(&fixture, document.document_id)
        .await
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n");
    let overlap_hash = normalize_record(SourceRecord {
        external_id: "not-hashed".into(),
        title: "Metadata Document".into(),
        summary: None,
        body_text: joined,
        source_uri: "context69://test".into(),
        published_at: None,
        updated_at: chrono::Utc::now(),
        metadata_json: stored_metadata,
    })
    .record_hash;
    assert_ne!(
        record_hash, overlap_hash,
        "reassembling overlapping chunks must not reproduce the stored hash"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_missing_current_version_rolls_the_file_and_every_document_back() {
    let fixture = MetadataFixture::new().await;
    let first = fixture
        .seed_document("first", 0, "first body", &["first body"])
        .await;
    let second = fixture
        .seed_document("second", 1, "second body", &["second body"])
        .await;
    fixture
        .delete_current_version(second.document_id, &second.record_hash)
        .await;
    let file_before = fixture.file_record().await;

    let error = fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect_err("a document without a current version must refuse publication");
    assert!(
        error.to_string().contains("no current content version"),
        "unexpected error: {error:#}"
    );

    let file_after = fixture.file_record().await;
    assert_eq!(
        file_before.metadata_json, file_after.metadata_json,
        "the file metadata must roll back with the document set"
    );
    assert_eq!(
        fixture.document_hash(first.document_id).await,
        first.record_hash,
        "the first document must roll back after the later document fails"
    );
    assert_eq!(
        fixture.document_hash(second.document_id).await,
        second.record_hash
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_stale_expected_hash_refuses_the_compare_and_set() {
    let fixture = MetadataFixture::new().await;
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    let payload = ChunkPayload {
        chunk_id: Uuid::nil(),
        document_id: document.document_id,
        group_id: fixture.group.id,
        group_key: fixture.group.key.clone(),
        group_path: fixture.group.path.clone(),
        visibility: Visibility::Private,
        source_key: "library".into(),
        external_id: "cas-payload".into(),
        title: "Metadata Document".into(),
        summary: None,
        source_uri: "context69://test".into(),
        published_at: None,
        updated_at_source: chrono::Utc::now(),
        record_hash: "cas-new-hash".into(),
        chunk_index: 0,
        chunk_text: CANONICAL_BODY.into(),
        metadata_json: serde_json::json!({}),
        content_locale: "original".into(),
        source_locale: None,
        translation_provider: None,
    };

    let mut tx = fixture.db.pool().begin().await.expect("begin");
    let applied = fixture
        .db
        .update_library_document_business_fields_in(
            &mut tx,
            document.document_id,
            &payload,
            "a-stale-baseline",
            CANONICAL_BODY,
        )
        .await
        .expect("the CAS reports a miss instead of an error");
    assert!(!applied, "a mismatched baseline must not be written");
    tx.rollback().await.expect("rollback");

    assert_eq!(
        fixture.document_hash(document.document_id).await,
        document.record_hash
    );
    assert_eq!(
        fixture
            .version_body(document.document_id, "cas-new-hash")
            .await,
        None
    );
    assert_eq!(
        stored_chunks(&fixture, document.document_id).await,
        document
            .chunk_ids
            .iter()
            .copied()
            .zip(document.chunk_texts.iter().cloned())
            .collect::<Vec<_>>(),
        "chunks and their hashes stay untouched on a CAS miss"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn metadata_edits_keep_document_identity_history_chunks_and_results() {
    let fixture = MetadataFixture::new().await;
    let document = fixture
        .seed_document(
            "section-one",
            0,
            CANONICAL_BODY,
            &["alpha beta gamma", "gamma delta epsilon"],
        )
        .await;
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, 'Metadata Author', 'unused') RETURNING id",
    )
    .bind(format!("metadata-{}", Uuid::new_v4()))
    .fetch_one(fixture.db.pool())
    .await
    .expect("seed user");
    sqlx::query(
        "INSERT INTO context69.document_results \
         (document_id, author_user_id, producer, result_kind, schema_ref, \
          source_record_hash, idempotency_key, result, provenance) \
         VALUES ($1, $2, 'test', 'summary', 'test/summary', $3, $4, '{\"text\":\"kept\"}', NULL)",
    )
    .bind(document.document_id)
    .bind(user_id)
    .bind(&document.record_hash)
    .bind(format!("result-{}", Uuid::new_v4()))
    .execute(fixture.db.pool())
    .await
    .expect("seed result");
    let chunks_before = stored_chunks(&fixture, document.document_id).await;

    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("first metadata publish");
    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v2"))
        .await
        .expect("second metadata publish");

    assert_eq!(
        fixture.version_count(document.document_id).await,
        3,
        "the seed version and both metadata versions are retained"
    );
    assert_eq!(
        stored_chunks(&fixture, document.document_id).await,
        chunks_before,
        "original chunks and their ids are never rewritten"
    );
    let result_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.document_results WHERE document_id = $1",
    )
    .bind(document.document_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("count results");
    assert_eq!(result_count, 1, "document results survive metadata edits");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_move_updates_the_file_and_the_document_path_together() {
    let fixture = MetadataFixture::new().await;
    let source_folder = fixture.create_folder("Source").await;
    let target_folder = fixture.create_folder("Target").await;
    fixture
        .store
        .move_file(fixture.file_id, Some(source_folder))
        .await
        .expect("place the file in the source folder");
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;

    let moved = fixture
        .service
        .move_file(
            fixture.file_id,
            &MoveFileRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect("move the file");

    assert_eq!(moved.folder_id, Some(target_folder));
    let stored = fixture.file_record().await;
    assert_eq!(stored.folder_id, Some(target_folder));
    let document_metadata = fixture.document_metadata(document.document_id).await;
    assert_eq!(
        document_metadata["library_path"],
        serde_json::json!("/Target"),
        "the document path is published in the same transaction as the move"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failed_move_rolls_the_folder_change_back() {
    let fixture = MetadataFixture::new().await;
    let source_folder = fixture.create_folder("Source").await;
    let target_folder = fixture.create_folder("Target").await;
    fixture
        .store
        .move_file(fixture.file_id, Some(source_folder))
        .await
        .expect("place the file in the source folder");
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    fixture
        .delete_current_version(document.document_id, &document.record_hash)
        .await;

    fixture
        .service
        .move_file(
            fixture.file_id,
            &MoveFileRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect_err("a move whose metadata refresh fails must roll back");

    let stored = fixture.file_record().await;
    assert_eq!(
        stored.folder_id,
        Some(source_folder),
        "the file stays in the source folder when the metadata refresh fails"
    );
    assert_eq!(
        fixture.document_hash(document.document_id).await,
        document.record_hash
    );
}

/// A metadata edit that moves the source hash retires the translation points
/// the new hash invalidates, in the commit that moves the hash.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_metadata_edit_retires_the_stale_translation_points() {
    let fixture = MetadataFixture::new_with_caps(Some(CAPS)).await;
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    let translation = fixture
        .seed_translation(
            document.document_id,
            "zh",
            &document.record_hash,
            &[(Uuid::new_v4(), "translated body".into())],
        )
        .await;

    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("publish metadata");

    assert_ne!(
        fixture.document_hash(document.document_id).await,
        document.record_hash,
        "the metadata edit moved the source hash"
    );

    let tombstones = fixture.registry_tombstones(translation[0]).await;
    assert_eq!(
        tombstones.len(),
        1,
        "the stale translation point is tombstoned exactly once"
    );
    assert_eq!(tombstones[0].phase, "retired");
    assert!(
        tombstones[0].staged_ids.is_empty(),
        "a retired tombstone stages no point"
    );
    assert_eq!(
        fixture
            .translation_version_count(document.document_id)
            .await,
        1,
        "the translation version row stays queryable"
    );
}

/// A refused admission rolls the file and every document back, so the source
/// hash never moves without the matching tombstone.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cap_refusal_rolls_the_file_and_the_documents_back() {
    let fixture = MetadataFixture::new_with_caps(Some([1, 1, 1])).await;
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    let translation = fixture
        .seed_translation(
            document.document_id,
            "zh",
            &document.record_hash,
            &[
                (Uuid::new_v4(), "translated one".into()),
                (Uuid::new_v4(), "translated two".into()),
            ],
        )
        .await;
    let file_before = fixture.file_record().await;

    let error = fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect_err("the admission refuses the tombstone within the given caps");
    assert!(
        error.to_string().contains("registry capacity policy"),
        "unexpected error: {error:#}"
    );

    assert_eq!(
        fixture.file_record().await.metadata_json,
        file_before.metadata_json,
        "the file metadata rolls back with the refused tombstone"
    );
    assert_eq!(
        fixture.document_hash(document.document_id).await,
        document.record_hash,
        "the document hash rolls back with the refused tombstone"
    );
    assert!(
        fixture.registry_tombstones(translation[0]).await.is_empty(),
        "a refused admission leaves no tombstone"
    );
}

/// Without the explicit caps the publisher keeps its disabled behavior and
/// retires no translation point.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn without_caps_the_metadata_edit_retires_no_translation_point() {
    let fixture = MetadataFixture::new_with_caps(None).await;
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    let translation = fixture
        .seed_translation(
            document.document_id,
            "zh",
            &document.record_hash,
            &[(Uuid::new_v4(), "translated body".into())],
        )
        .await;

    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("publish metadata");

    assert_ne!(
        fixture.document_hash(document.document_id).await,
        document.record_hash,
        "the metadata edit moved the source hash"
    );
    assert!(
        fixture.registry_tombstones(translation[0]).await.is_empty(),
        "the disabled publisher creates no tombstone"
    );
}

/// A second metadata edit invalidates the same translation once more. The
/// bounded admission must not fail on an id a registry entry already
/// excludes, and the translation stays tombstoned exactly once.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_second_metadata_edit_keeps_the_tombstoned_translation_quiet() {
    let fixture = MetadataFixture::new_with_caps(Some(CAPS)).await;
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    let translation = fixture
        .seed_translation(
            document.document_id,
            "zh",
            &document.record_hash,
            &[(Uuid::new_v4(), "translated body".into())],
        )
        .await;

    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("the first metadata publish");
    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v2"))
        .await
        .expect("the second metadata publish");

    assert_eq!(
        fixture.registry_tombstones(translation[0]).await.len(),
        1,
        "the already excluded translation is not tombstoned twice"
    );
}
