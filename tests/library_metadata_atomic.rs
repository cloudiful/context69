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

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

use context69::contracts::{CanonicalUploadMetadata, MoveFileRequest, MoveFolderRequest};
use context69::db::{Database, LibraryMetadataMirrorLock};
use context69::domain::{ChunkPayload, SourceRecord};
use context69::normalize::normalize_record;
use context69::qdrant_index::QdrantIndex;
use context69::services::metadata_payload_sync::MetadataPayloadSyncWorker;
use context69_contracts::Visibility;
use qdrant_points_mock::QdrantPointsMock;
use sqlx::Row;
use support::{
    CANONICAL_BODY, MIRROR_DIMENSIONS, MetadataFixture, MirrorFixture, can_take_session_lock,
    mirror_runtime, wait_for_session_lock_release,
};
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

async fn hold_rebuild_barrier(fixture: &MetadataFixture) {
    let mut tx = fixture
        .db
        .pool()
        .begin()
        .await
        .expect("begin the barrier claim");
    let outcome = fixture
        .db
        .begin_rebuild_in(
            &mut tx,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "held-fingerprint",
            "held-target",
        )
        .await
        .expect("claim the rebuild barrier");
    assert!(matches!(
        outcome,
        context69::db::RebuildStartOutcome::Started(_)
    ));
    tx.commit().await.expect("commit the barrier claim");
}

async fn clear_rebuild_barrier(fixture: &MetadataFixture) {
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
             rebuild_fingerprint = NULL, rebuild_collection = NULL, \
             rebuild_started_at = NULL, rebuild_error = NULL \
         WHERE id = 1",
    )
    .execute(fixture.db.pool())
    .await
    .expect("clear the fixture barrier");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_untranslated_file_move_refuses_under_the_rebuild_barrier_and_succeeds_when_idle() {
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
    let hash_before = fixture.document_hash(document.document_id).await;
    let path_before = fixture.document_metadata(document.document_id).await["library_path"].clone();
    hold_rebuild_barrier(&fixture).await;

    fixture
        .service
        .move_file(
            fixture.file_id,
            &MoveFileRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect_err("an ordinary move must not bypass the rebuild barrier");

    assert_eq!(
        fixture.file_record().await.folder_id,
        Some(source_folder),
        "a refused move commits no folder change"
    );
    assert_eq!(
        fixture.document_hash(document.document_id).await,
        hash_before,
        "a refused move commits no canonical hash change"
    );
    assert_eq!(
        fixture.document_metadata(document.document_id).await["library_path"],
        path_before,
        "a refused move commits no canonical path change"
    );

    clear_rebuild_barrier(&fixture).await;
    fixture
        .service
        .move_file(
            fixture.file_id,
            &MoveFileRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect("the untranslated move succeeds when the barrier is idle");
    assert_eq!(
        fixture.file_record().await.folder_id,
        Some(target_folder),
        "the idle move commits the folder change"
    );
    assert_eq!(
        fixture.document_metadata(document.document_id).await["library_path"],
        serde_json::json!("/Target"),
        "the idle move commits the descendant path with the folder"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_folder_move_refuses_under_the_rebuild_barrier_without_a_partial_subtree() {
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
    let hash_before = fixture.document_hash(document.document_id).await;
    let path_before = fixture.document_metadata(document.document_id).await["library_path"].clone();
    hold_rebuild_barrier(&fixture).await;

    fixture
        .service
        .move_folder(
            source_folder,
            &MoveFolderRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect_err("a folder move must not bypass the rebuild barrier");

    let folder = fixture
        .store
        .get_folder(source_folder)
        .await
        .expect("read the folder")
        .expect("the folder still exists");
    assert_eq!(
        folder.parent_id, None,
        "the refused folder move commits no parent change"
    );
    assert_eq!(
        fixture.file_record().await.folder_id,
        Some(source_folder),
        "the descendant file stays in the source folder"
    );
    assert_eq!(
        fixture.document_hash(document.document_id).await,
        hash_before,
        "the refused folder move commits no descendant hash change"
    );
    assert_eq!(
        fixture.document_metadata(document.document_id).await["library_path"],
        path_before,
        "the refused folder move commits no partial descendant path"
    );

    clear_rebuild_barrier(&fixture).await;
    fixture
        .service
        .move_folder(
            source_folder,
            &MoveFolderRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect("the folder move succeeds when the barrier is idle");
    assert_eq!(
        fixture.document_metadata(document.document_id).await["library_path"],
        serde_json::json!("/Target/Source"),
        "the idle folder move refreshes the whole subtree"
    );
}

/// Production always installs registry caps, but caps do not prove that a
/// vector runtime was ever published. A desired configuration awaiting its
/// first index still permits unindexed file and empty folder moves, and no
/// remote payload mirror exists without an active pointer.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn production_caps_without_a_vector_runtime_move_metadata_only() {
    let fixture = MetadataFixture::new_with_caps(Some(CAPS)).await;
    fixture
        .seed_desired_vector_settings(
            "http://qdrant.test:6334",
            "desired-collection",
            "http://embedding.test",
            "desired-model",
            4,
        )
        .await;
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
        .expect("an unindexed file move must not require a vector runtime");
    assert_eq!(moved.folder_id, Some(target_folder));
    assert_eq!(
        fixture.document_metadata(document.document_id).await["library_path"],
        serde_json::json!("/Target")
    );

    let empty_folder = fixture.create_folder("Empty").await;
    let moved_folder = fixture
        .service
        .move_folder(
            empty_folder,
            &MoveFolderRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect("an empty folder move must not require a vector runtime");
    assert_eq!(moved_folder.parent_folder_id, Some(target_folder));

    let active: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.vector_index_state WHERE active")
            .fetch_one(fixture.db.pool())
            .await
            .expect("count the active vector state");
    assert_eq!(
        active, 0,
        "the metadata-only move invents no active vector target"
    );
}

/// An active index whose logical identity differs from the desired settings
/// refuses business metadata and refresh even when the publication has no
/// translations to retire. The same mismatch is refused by the shared
/// identity guard, not only by the translation-retirement path.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_active_desired_mismatch_refuses_metadata_and_refresh() {
    let fixture = MetadataFixture::new_with_caps(Some(CAPS)).await;
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    fixture
        .seed_desired_vector_settings(
            "http://qdrant.desired.test:6334",
            "desired-collection",
            "http://embedding.desired.test",
            "desired-model",
            4,
        )
        .await;
    fixture
        .seed_active_vector_state("active-collection", "active-fingerprint", 4)
        .await;
    let file_before = fixture.file_record().await;
    let hash_before = fixture.document_hash(document.document_id).await;

    let error = fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect_err("an active mismatched vector index refuses the publication");
    assert!(
        error.to_string().contains("vector"),
        "unexpected error: {error:#}"
    );
    assert_eq!(
        fixture.file_record().await.metadata_json,
        file_before.metadata_json,
        "the refused publication commits no file metadata"
    );
    assert_eq!(
        fixture.document_hash(document.document_id).await,
        hash_before,
        "the refused publication commits no canonical hash"
    );

    let error = fixture
        .service
        .refresh_metadata_for_file_for_test(fixture.file_id)
        .await
        .expect_err("the refresh refuses across the same active mismatch");
    assert!(
        error.to_string().contains("vector"),
        "unexpected refresh error: {error:#}"
    );

    let mut tx = fixture.db.pool().begin().await.expect("begin");
    let captured = context69::db::RegistryVectorIdentity {
        embedding_fingerprint: "active-fingerprint".into(),
        collection_name: "active-collection".into(),
    };
    let error = fixture
        .db
        .require_idle_admissible_vector_identity_in(&mut tx, Some(&captured))
        .await
        .expect_err("the desired identity differs from the active pointer");
    assert!(
        error
            .to_string()
            .contains("desired vector configuration differs"),
        "unexpected identity error: {error:#}"
    );
    tx.rollback().await.expect("rollback");

    fixture.clear_active_vector_state().await;
}

/// A descendant canonical refresh failure rolls the entire folder move back:
/// the source parent, the descendant file folder and the document hash keep
/// the state they had before the move.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_folder_move_refresh_failure_rolls_the_whole_subtree_back() {
    let fixture = MetadataFixture::new().await;
    let source_folder = fixture.create_folder("Source").await;
    let child_folder = fixture.create_child_folder(source_folder, "Child").await;
    let target_folder = fixture.create_folder("Target").await;
    fixture
        .store
        .move_file(fixture.file_id, Some(child_folder))
        .await
        .expect("place the file in the child folder");
    let document = fixture
        .seed_document("section-one", 0, CANONICAL_BODY, &[CANONICAL_BODY])
        .await;
    fixture
        .delete_current_version(document.document_id, &document.record_hash)
        .await;

    fixture
        .service
        .move_folder(
            source_folder,
            &MoveFolderRequest {
                target_folder_id: Some(target_folder),
            },
        )
        .await
        .expect_err("a failed descendant refresh must roll the folder move back");

    let source = fixture
        .store
        .get_folder(source_folder)
        .await
        .expect("read the source folder")
        .expect("the source folder exists");
    assert_eq!(
        source.parent_id, None,
        "the source folder keeps its parent when a descendant fails"
    );
    let child = fixture
        .store
        .get_folder(child_folder)
        .await
        .expect("read the child folder")
        .expect("the child folder exists");
    assert_eq!(child.parent_id, Some(source_folder));
    assert_eq!(
        fixture.file_record().await.folder_id,
        Some(child_folder),
        "the descendant file keeps its folder"
    );
    assert_eq!(
        fixture.document_hash(document.document_id).await,
        document.record_hash,
        "the descendant document keeps its canonical hash"
    );
}

/// Two opposing moves serialize on the epoch lock: the second revalidates the
/// current tree and refuses, so neither a stored cycle nor an unbounded
/// ancestor walk can result from the race. A pre-existing malformed cycle
/// still terminates.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn opposing_folder_moves_cannot_save_a_cycle() {
    let fixture = MetadataFixture::new().await;
    let first = fixture.create_folder("First").await;
    let second = fixture.create_folder("Second").await;

    let service = fixture.service.clone();
    let first_request = MoveFolderRequest {
        target_folder_id: Some(second),
    };
    let second_request = MoveFolderRequest {
        target_folder_id: Some(first),
    };
    let first_to_second = service.move_folder(first, &first_request);
    let second_to_first = service.move_folder(second, &second_request);
    let (first_result, second_result) = tokio::join!(first_to_second, second_to_first);
    assert!(
        first_result.is_ok() ^ second_result.is_ok(),
        "exactly one opposing move may win: {first_result:?} / {second_result:?}"
    );

    let first_folder = fixture
        .store
        .get_folder(first)
        .await
        .expect("read first")
        .expect("first exists");
    let second_folder = fixture
        .store
        .get_folder(second)
        .await
        .expect("read second")
        .expect("second exists");
    assert!(
        !(first_folder.parent_id == Some(second) && second_folder.parent_id == Some(first)),
        "the two moves must not save a cycle"
    );

    sqlx::query("UPDATE context69.library_folders SET parent_id = $1 WHERE id = $2")
        .bind(second)
        .bind(first)
        .execute(fixture.db.pool())
        .await
        .expect("create a malformed cycle");
    sqlx::query("UPDATE context69.library_folders SET parent_id = $1 WHERE id = $2")
        .bind(first)
        .bind(second)
        .execute(fixture.db.pool())
        .await
        .expect("close the malformed cycle");
    let mut connection = fixture.db.pool().acquire().await.expect("acquire");
    let ancestors = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.store.folder_ancestor_ids_on(&mut connection, first),
    )
    .await
    .expect("the cycle-bounded ancestor query must terminate")
    .expect("read the ancestor set");
    assert!(
        ancestors.len() <= 2,
        "distinct-set recursion visits a malformed cycle once per node"
    );
    drop(connection);
    sqlx::query("UPDATE context69.library_folders SET parent_id = NULL WHERE id = ANY($1)")
        .bind(vec![first, second])
        .execute(fixture.db.pool())
        .await
        .expect("repair the malformed cycle");
}

/// A captured older publication cannot overwrite a newer commit: the first
/// worker holds the per-file session lock and blocks its send, the second
/// enqueue commits, the second worker cannot enter the send until the first
/// releases, and the final visible payload is the newer canonical state.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_paused_older_mirror_cannot_overwrite_a_newer_commit() {
    let mock = QdrantPointsMock::start().await;
    let index = QdrantIndex::for_test_unreachable(mock.url(), "mirror-a", MIRROR_DIMENSIONS)
        .expect("build the mirror index");
    let fixture = MirrorFixture::new(index, mock.url()).await;
    let document = fixture
        .seed_document("body", CANONICAL_BODY, &["alpha beta", "gamma delta"])
        .await;
    for chunk_id in &document.chunk_ids {
        mock.insert_preexisting_library_point(
            &chunk_id.to_string(),
            fixture.file_id,
            document.document_id,
            &document.record_hash,
            MIRROR_DIMENSIONS,
        );
    }
    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("publish v1");
    assert_eq!(fixture.marker().await, Some((1, 0)));

    let gate = mock.pause_set_payload();
    let worker_a = fixture.worker.clone();
    let task_a = tokio::spawn(async move { worker_a.drain_mirror_once_for_test().await });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while mock.set_payload_entered() == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the first mirror must reach its payload send"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v2"))
        .await
        .expect("publish v2 while the older mirror is paused");
    assert_eq!(fixture.marker().await, Some((2, 0)));

    fixture
        .worker
        .drain_mirror_once_for_test()
        .await
        .expect("the second worker pass runs");
    assert_eq!(
        mock.set_payload_entered(),
        1,
        "the second worker never enters a payload send while the first owns the session lock"
    );
    assert!(
        mock.set_payload_calls().is_empty(),
        "no payload is applied before the paused send is released"
    );

    gate.add_permits(1);
    mock.resume_set_payload();
    task_a
        .await
        .expect("the first worker task joins")
        .expect("the first worker pass completes");
    assert_eq!(
        fixture.marker().await,
        Some((2, 0)),
        "the stale completion leaves the newer enqueue pending"
    );

    fixture.make_marker_due().await;
    let restarted_worker =
        MetadataPayloadSyncWorker::new(fixture.db.clone(), fixture.runtime.clone());
    restarted_worker
        .drain_mirror_once_for_test()
        .await
        .expect("the newer marker drains from a fresh worker instance");
    assert_eq!(fixture.marker().await, None, "the marker is drained");

    let calls = mock.set_payload_calls();
    assert_eq!(calls.len(), 2, "both serialized sends are observable");
    let last = calls.last().expect("the newer send exists");
    assert_eq!(last.collection, "mirror-a");
    let updated = last
        .updates
        .iter()
        .find(|update| update.point_id == document.chunk_ids[0].to_string())
        .expect("the chunk was updated");
    assert_eq!(
        updated
            .payload
            .get("metadata_index")
            .and_then(|value| qdrant_points_mock::struct_string_field(value, "tag"))
            .as_deref(),
        Some("v2"),
        "the final visible payload carries the newer canonical metadata"
    );

    fixture.cleanup().await;
}

/// A cancelled send never leaves the per-file session lock behind: aborting the
/// worker while the external call is in flight drops the dedicated session, so a
/// fresh session can take the file's lock and the marker stays pending.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancelled_mirror_leaves_no_session_lock() {
    let mock = QdrantPointsMock::start().await;
    let index = QdrantIndex::for_test_unreachable(mock.url(), "mirror-cancel", MIRROR_DIMENSIONS)
        .expect("build the mirror index");
    let fixture = MirrorFixture::new(index, mock.url()).await;
    let document = fixture
        .seed_document("body", CANONICAL_BODY, &["alpha beta", "gamma delta"])
        .await;
    for chunk_id in &document.chunk_ids {
        mock.insert_preexisting_library_point(
            &chunk_id.to_string(),
            fixture.file_id,
            document.document_id,
            &document.record_hash,
            MIRROR_DIMENSIONS,
        );
    }
    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("publish v1");

    let gate = mock.pause_set_payload();
    let worker = fixture.worker.clone();
    let task = tokio::spawn(async move { worker.drain_mirror_once_for_test().await });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while mock.set_payload_entered() == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the mirror must reach its payload send before it can be cancelled"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(
        task.await
            .expect_err("the task is cancelled")
            .is_cancelled()
    );

    let release_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !fixture.can_take_session_lock(fixture.file_id).await {
        assert!(
            tokio::time::Instant::now() < release_deadline,
            "the cancelled mirror must release its per-file session lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        fixture.marker().await,
        Some((1, 0)),
        "a cancelled send leaves the marker pending for the next pass"
    );

    gate.add_permits(1);
    mock.resume_set_payload();
    fixture.cleanup().await;
}

/// The PostgreSQL backend session of one connection.
async fn backend_session_id(connection: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_file_scalar!("tests/library_metadata_atomic/read_backend_session_id.sql")
        .fetch_one(connection)
        .await
        .expect("read the backend session id")
        .expect("the backend session id is never null")
}

/// The PostgreSQL backend session the pool currently hands out.
async fn pooled_backend_session_id(db: &Database) -> i32 {
    let mut connection = db
        .pool()
        .acquire()
        .await
        .expect("acquire the pooled session");
    let session_id = backend_session_id(&mut connection).await;
    drop(connection);
    session_id
}

/// Wait until the dedicated pool holds exactly one returned session.
///
/// The pool returns a session to its idle queue asynchronously, so the
/// observation waits for the pool to settle instead of reading its counters
/// right after the session was released.
async fn wait_for_one_returned_session(db: &Database) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while db.pool().num_idle() != 1 || db.pool().size() != 1 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the pool never settled on the one returned session"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Cancelling after PostgreSQL took the session lock but before acquisition
/// returned must detach that session instead of returning it to the pool with
/// the lock still held. The armed window parks the exact guarded connection
/// after the real advisory statement succeeded.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancelled_lock_acquisition_never_returns_a_locked_session_to_the_pool() {
    let fixture = MetadataFixture::new().await;
    let file_id = fixture.file_id;
    LibraryMetadataMirrorLock::arm_acquisition_cancellation_window(file_id);
    let db = fixture.db.clone();
    let task = tokio::spawn(async move { db.try_lock_library_metadata_mirror(file_id).await });
    let reached = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        LibraryMetadataMirrorLock::wait_acquisition_cancellation_window(file_id),
    )
    .await;
    if reached.is_err() {
        task.abort();
        LibraryMetadataMirrorLock::clear_cancellation_windows();
        wait_for_session_lock_release(file_id).await;
        panic!("the parked acquisition never reached its cancellation window");
    }
    assert!(
        !can_take_session_lock(file_id).await,
        "the parked acquisition holds the real session lock"
    );

    task.abort();
    let join = task.await;
    assert!(
        matches!(join, Err(error) if error.is_cancelled()),
        "the acquisition is cancelled"
    );
    LibraryMetadataMirrorLock::clear_cancellation_windows();
    wait_for_session_lock_release(file_id).await;
    fixture.cleanup().await;
}

/// Cancelling an explicit release after the guard is parked but before the
/// unlock statement is submitted must also detach the held session. A healthy
/// unlock afterwards returns its connection to the pool instead of closing it.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancelled_explicit_release_never_returns_a_locked_session_to_the_pool() {
    let fixture = MetadataFixture::new().await;
    let file_id = fixture.file_id;
    let lock = fixture
        .db
        .try_lock_library_metadata_mirror(file_id)
        .await
        .expect("acquire the mirror lock")
        .expect("the file lock is free");
    LibraryMetadataMirrorLock::arm_release_cancellation_window(file_id);
    let task = tokio::spawn(async move { lock.release().await });
    let reached = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        LibraryMetadataMirrorLock::wait_release_cancellation_window(file_id),
    )
    .await;
    if reached.is_err() {
        task.abort();
        LibraryMetadataMirrorLock::clear_cancellation_windows();
        wait_for_session_lock_release(file_id).await;
        panic!("the parked release never reached its cancellation window");
    }
    assert!(
        !can_take_session_lock(file_id).await,
        "the parked release still holds the session lock"
    );

    task.abort();
    let join = task.await;
    assert!(
        matches!(join, Err(error) if error.is_cancelled()),
        "the release is cancelled"
    );
    LibraryMetadataMirrorLock::clear_cancellation_windows();
    wait_for_session_lock_release(file_id).await;

    // A healthy unlock returns its session to the pool instead of closing it.
    // The dedicated pool settles on one backend session, so the session it
    // hands out before the lock and after the healthy release is the session
    // the lock held; a release that closed the connection would leave the
    // pool with a different backend.
    let pool = Database::connect_read_only(&context69_test_support::accessor::database_url())
        .await
        .expect("open the dedicated pool");
    let before = pooled_backend_session_id(&pool).await;
    wait_for_one_returned_session(&pool).await;
    let mut lock = pool
        .try_lock_library_metadata_mirror(file_id)
        .await
        .expect("acquire the healthy lock")
        .expect("the file lock is free again");
    let locked = backend_session_id(lock.connection()).await;
    assert_eq!(
        locked, before,
        "the healthy lock runs on the pooled session"
    );
    lock.release().await;
    wait_for_one_returned_session(&pool).await;
    let after = pooled_backend_session_id(&pool).await;
    assert_eq!(
        after, locked,
        "a healthy unlock returns the same session to the pool instead of closing it"
    );

    fixture.cleanup().await;
}

/// A failed send stays pending under the existing retry curve; a newer enqueue
/// resets the backoff and is the version that finally completes. No session
/// lock survives the failed attempt.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failed_mirror_send_retries_without_losing_a_newer_enqueue() {
    let mock = QdrantPointsMock::start().await;
    let index = QdrantIndex::for_test_unreachable(mock.url(), "mirror-retry", MIRROR_DIMENSIONS)
        .expect("build the mirror index");
    let fixture = MirrorFixture::new(index, mock.url()).await;
    let document = fixture
        .seed_document("body", CANONICAL_BODY, &["alpha beta", "gamma delta"])
        .await;
    for chunk_id in &document.chunk_ids {
        mock.insert_preexisting_library_point(
            &chunk_id.to_string(),
            fixture.file_id,
            document.document_id,
            &document.record_hash,
            MIRROR_DIMENSIONS,
        );
    }

    let worker = fixture.worker.clone();
    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("publish v1");
    assert!(
        mock.set_payload_calls().is_empty(),
        "the canonical metadata transaction never sends a payload"
    );
    mock.fail_set_payload_once();
    assert_eq!(
        mock.pending_payload_failures(),
        1,
        "the one-shot failure is armed before the pass"
    );
    assert!(
        !fixture
            .store
            .list_chunk_payloads_for_files(&[fixture.file_id])
            .await
            .expect("read the canonical payloads")
            .is_empty(),
        "the mirror candidate has canonical payloads to send"
    );
    worker
        .drain_mirror_once_for_test()
        .await
        .expect("the failed send is a deferred candidate, not a pass failure");
    assert_eq!(
        mock.pending_payload_failures(),
        0,
        "the pass reached the payload send and consumed the armed failure"
    );
    assert!(
        mock.set_payload_calls().is_empty(),
        "the failed send applies no payload"
    );
    assert_eq!(
        fixture.marker().await,
        Some((1, 1)),
        "the failed version records one attempt and stays pending"
    );
    assert!(
        fixture.can_take_session_lock(fixture.file_id).await,
        "the failed attempt releases its per-file session lock"
    );

    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v2"))
        .await
        .expect("publish v2");
    assert_eq!(
        fixture.marker().await,
        Some((2, 0)),
        "the newer enqueue resets the attempt and the backoff"
    );
    worker
        .drain_mirror_once_for_test()
        .await
        .expect("the newer version drains");
    assert_eq!(fixture.marker().await, None);
    let calls = mock.set_payload_calls();
    assert_eq!(calls.len(), 1, "only the successful newer send is applied");
    let updated = calls[0]
        .updates
        .iter()
        .find(|update| update.point_id == document.chunk_ids[0].to_string())
        .expect("the newer send updates the chunk");
    let tag = updated
        .payload
        .get("metadata_index")
        .and_then(|value| qdrant_points_mock::struct_string_field(value, "tag"));
    assert_eq!(tag.as_deref(), Some("v2"));

    fixture.cleanup().await;
}

/// A published desired-only pair never receives a mirror payload. The marker
/// waits until the same pair becomes the active pointer, then mirrors exactly
/// into that collection.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_mirror_never_writes_a_desired_only_index() {
    let active_mock = QdrantPointsMock::start().await;
    let desired_mock = QdrantPointsMock::start().await;
    let active_index =
        QdrantIndex::for_test_unreachable(active_mock.url(), "mirror-active", MIRROR_DIMENSIONS)
            .expect("build the active mirror index");
    let fixture = MirrorFixture::new(active_index, active_mock.url()).await;
    let document = fixture
        .seed_document("body", CANONICAL_BODY, &["alpha beta", "gamma delta"])
        .await;
    for chunk_id in &document.chunk_ids {
        active_mock.insert_preexisting_library_point(
            &chunk_id.to_string(),
            fixture.file_id,
            document.document_id,
            &document.record_hash,
            MIRROR_DIMENSIONS,
        );
        desired_mock.insert_preexisting_library_point(
            &chunk_id.to_string(),
            fixture.file_id,
            document.document_id,
            &document.record_hash,
            MIRROR_DIMENSIONS,
        );
    }
    fixture
        .service
        .apply_business_metadata_for_test(fixture.file_id, &metadata("v1"))
        .await
        .expect("publish v1");

    let desired_index =
        QdrantIndex::for_test_unreachable(desired_mock.url(), "mirror-desired", MIRROR_DIMENSIONS)
            .expect("build the desired mirror index");
    let (desired_runtime, desired_fingerprint) = mirror_runtime(desired_index, desired_mock.url());
    let desired_worker = MetadataPayloadSyncWorker::new(fixture.db.clone(), desired_runtime);
    desired_worker
        .drain_mirror_once_for_test()
        .await
        .expect("the desired-only pass runs");
    assert_eq!(
        fixture.marker().await,
        Some((1, 1)),
        "a pair that is not the active pointer is deferred"
    );
    assert!(
        desired_mock.set_payload_calls().is_empty(),
        "the desired-only index receives no payload"
    );
    assert!(
        active_mock.set_payload_calls().is_empty(),
        "the active index is not written by a pair it does not describe"
    );

    fixture
        .set_desired(desired_mock.url(), "mirror-desired")
        .await;
    fixture
        .set_active("mirror-desired", &desired_fingerprint)
        .await;
    fixture.make_marker_due().await;
    desired_worker
        .drain_mirror_once_for_test()
        .await
        .expect("the activated pair drains the marker");
    assert_eq!(fixture.marker().await, None);
    assert_eq!(
        desired_mock.set_payload_calls().len(),
        1,
        "the now-active collection receives exactly one payload publication"
    );
    assert!(
        active_mock.set_payload_calls().is_empty(),
        "the retired collection is never written"
    );

    fixture.cleanup().await;
}
