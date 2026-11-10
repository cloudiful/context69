//! Entity tombstones in the coherent search snapshot (issue #1138 P24).
//!
//! The cases prove that a deleted library file or an explicitly deleted
//! document leaves the search snapshot through its entity identity, that the
//! snapshot and the two generations advance together, that a rollback leaves
//! the snapshot untouched, that a purge never removes the exclusion, and that
//! a tombstoned id is never handed out again.

#[path = "deleted_entities/harness.rs"]
mod harness;

use anyhow::Result;
use context69::db::Database;
use harness::{
    FileFixture, GroupFixture, clear_tombstones, peek_next_document_id, registry_epoch,
    search_generation, tombstone_document_ids, tombstone_file, tombstone_ids,
};
use uuid::Uuid;

async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_deleted_file_enters_the_snapshot_and_advances_both_generations() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_id = Uuid::new_v4();

    let epoch_before = registry_epoch(&db).await.expect("epoch before");
    let generation_before = search_generation(&db).await.expect("generation before");

    let mut tx = db.pool().begin().await.expect("begin");
    let inserted = tombstone_file(&db, &mut tx, file_id, group.id)
        .await
        .expect("tombstone the file");
    assert!(inserted, "the first tombstone of a file is a new insertion");
    tx.commit().await.expect("commit");

    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        state.deleted_file_ids.contains(&file_id),
        "the deleted file must be part of the same snapshot as the epoch"
    );
    assert_eq!(
        state.epoch,
        epoch_before + 1,
        "a new tombstone advances the registry epoch exactly once"
    );
    assert_eq!(
        search_generation(&db).await.expect("generation"),
        generation_before + 1,
        "a new tombstone invalidates the cached search responses"
    );

    let ids = tombstone_ids(&db, &[file_id], &[])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_deleted_document_enters_the_snapshot_and_advances_both_generations() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let document_id = 900_000_000 + (Uuid::new_v4().as_u128() % 1_000) as i64;

    let epoch_before = registry_epoch(&db).await.expect("epoch before");
    let generation_before = search_generation(&db).await.expect("generation before");

    let mut tx = db.pool().begin().await.expect("begin");
    let inserted = db
        .insert_deleted_document_entity_in(&mut tx, document_id, Some(group.id))
        .await
        .expect("tombstone the document");
    assert!(
        inserted,
        "the first tombstone of a document is a new insertion"
    );
    tx.commit().await.expect("commit");

    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        state.deleted_document_ids.contains(&document_id),
        "the deleted document must be part of the same snapshot as the epoch"
    );
    assert_eq!(
        state.epoch,
        epoch_before + 1,
        "a new tombstone advances the registry epoch exactly once"
    );
    assert_eq!(
        search_generation(&db).await.expect("generation"),
        generation_before + 1
    );

    let ids = tombstone_ids(&db, &[], &[document_id])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_repeated_tombstone_advances_no_generation() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_id = Uuid::new_v4();

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        tombstone_file(&db, &mut tx, file_id, group.id)
            .await
            .expect("first tombstone")
    );
    tx.commit().await.expect("commit");

    let epoch_after_first = registry_epoch(&db).await.expect("epoch");
    let generation_after_first = search_generation(&db).await.expect("generation");

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        !tombstone_file(&db, &mut tx, file_id, group.id)
            .await
            .expect("second tombstone"),
        "a repeated tombstone of the same file is not a new insertion"
    );
    tx.commit().await.expect("commit");

    assert_eq!(
        registry_epoch(&db).await.expect("epoch"),
        epoch_after_first,
        "a repeated tombstone must not advance the epoch again"
    );
    assert_eq!(
        search_generation(&db).await.expect("generation"),
        generation_after_first,
        "a repeated tombstone must not invalidate the cache again"
    );

    let ids = tombstone_ids(&db, &[file_id], &[])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_rolled_back_tombstone_leaves_the_snapshot_untouched() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_id = Uuid::new_v4();

    let epoch_before = registry_epoch(&db).await.expect("epoch before");
    let generation_before = search_generation(&db).await.expect("generation before");

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        tombstone_file(&db, &mut tx, file_id, group.id)
            .await
            .expect("tombstone")
    );
    tx.rollback().await.expect("rollback");

    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        !state.deleted_file_ids.contains(&file_id),
        "a rolled back tombstone must never enter the snapshot"
    );
    assert_eq!(
        state.epoch, epoch_before,
        "a rolled back tombstone must not advance the epoch"
    );
    assert_eq!(
        search_generation(&db).await.expect("generation"),
        generation_before,
        "a rolled back tombstone must not invalidate the cache"
    );

    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_purged_tombstone_still_excludes_the_entity() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_id = Uuid::new_v4();

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        tombstone_file(&db, &mut tx, file_id, group.id)
            .await
            .expect("tombstone")
    );
    tx.commit().await.expect("commit");

    let ids = tombstone_ids(&db, &[file_id], &[])
        .await
        .expect("tombstone ids");
    assert_eq!(ids.len(), 1, "the tombstone is one row");

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        db.mark_deleted_entity_purged_in(&mut tx, ids[0], None, None)
            .await
            .expect("mark purged"),
        "the first purge stamps the observation"
    );
    assert!(
        !db.mark_deleted_entity_purged_in(&mut tx, ids[0], None, None)
            .await
            .expect("mark purged twice"),
        "a repeated purge stamps nothing"
    );
    tx.commit().await.expect("commit");

    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        state.deleted_file_ids.contains(&file_id),
        "a purged tombstone still excludes its entity"
    );
    let pending = db
        .list_pending_deleted_entities(100)
        .await
        .expect("pending list");
    assert!(
        !pending.iter().any(|entity| entity.id == ids[0]),
        "a purged tombstone leaves the pending list"
    );

    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_pending_list_is_bounded() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_ids = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];

    let mut tx = db.pool().begin().await.expect("begin");
    for file_id in &file_ids {
        assert!(
            tombstone_file(&db, &mut tx, *file_id, group.id)
                .await
                .expect("tombstone")
        );
    }
    tx.commit().await.expect("commit");

    let pending = db.list_pending_deleted_entities(2).await.expect("pending");
    assert_eq!(
        pending.len(),
        2,
        "the pending list must respect the caller's bound"
    );

    let ids = tombstone_ids(&db, &file_ids, &[])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn deleting_a_library_file_tombstones_it_through_the_backstop() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file = FileFixture::create(&db, group.id)
        .await
        .expect("create file");

    let epoch_before = registry_epoch(&db).await.expect("epoch before");
    let generation_before = search_generation(&db).await.expect("generation before");

    file.remove(&db).await.expect("delete the file");

    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        state.deleted_file_ids.contains(&file.id),
        "the delete backstop must tombstone the deleted file"
    );
    assert_eq!(
        state.epoch,
        epoch_before + 1,
        "the delete backstop must advance the registry epoch"
    );
    assert_eq!(
        search_generation(&db).await.expect("generation"),
        generation_before + 1,
        "the delete backstop must invalidate the cached search responses"
    );

    let ids = tombstone_ids(&db, &[file.id], &[])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_tombstoned_file_id_cannot_be_reused() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_id = Uuid::new_v4();

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        tombstone_file(&db, &mut tx, file_id, group.id)
            .await
            .expect("tombstone")
    );
    tx.commit().await.expect("commit");

    let reused = sqlx::query(
        "INSERT INTO context69.library_files (id, filename, media_type, size_bytes, sha256, \
         storage_rel_path, ingest_status, group_id, visibility, external_id) \
         VALUES ($1, 'reused.txt', 'text/plain', 1, 'sha', 'p24/reused.txt', 'pending', $2, \
         'public', 'reused')",
    )
    .bind(file_id)
    .bind(group.id)
    .execute(db.pool())
    .await;
    assert!(
        reused.is_err(),
        "a tombstoned file id must never be handed out again"
    );

    let ids = tombstone_ids(&db, &[file_id], &[])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_tombstoned_document_id_cannot_be_reused() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let document_id = 800_000_000 + (Uuid::new_v4().as_u128() % 1_000) as i64;

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        db.insert_deleted_document_entity_in(&mut tx, document_id, Some(group.id))
            .await
            .expect("tombstone")
    );
    tx.commit().await.expect("commit");

    let reused = sqlx::query(
        "INSERT INTO context69.documents (id, source_key, external_id, title, source_uri, \
         updated_at_source, record_hash, group_id, visibility) \
         VALUES ($1, 'p24', $2, 'reused', 'urn:p24:reused', now(), 'hash', $3, 'public')",
    )
    .bind(document_id)
    .bind(format!("p24-reused-{document_id}"))
    .bind(group.id)
    .execute(db.pool())
    .await;
    assert!(
        reused.is_err(),
        "a tombstoned document id must never be handed out again"
    );

    let ids = tombstone_ids(&db, &[], &[document_id])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_reserved_document_id_skips_a_tombstoned_sequence_value() {
    let db = connect_scratch().await;

    let anchor = peek_next_document_id(&db).await.expect("peek the sequence");
    let tombstoned: Vec<i64> = (1..=4).map(|offset| anchor + offset).collect();
    let ids = tombstone_document_ids(&db, &tombstoned)
        .await
        .expect("tombstone the run");

    let mut tx = db.pool().begin().await.expect("begin");
    let reserved = db.reserve_document_id_on(&mut tx).await.expect("reserve");
    tx.rollback().await.expect("rollback");

    assert!(
        reserved > *tombstoned.last().expect("the tombstoned run"),
        "the reservation must skip every tombstoned sequence value, got {reserved}"
    );

    clear_tombstones(&db, &ids).await.expect("clear");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_reserved_document_id_refuses_a_run_of_tombstoned_values() {
    let db = connect_scratch().await;

    let anchor = peek_next_document_id(&db).await.expect("peek the sequence");
    let tombstoned: Vec<i64> = (1..=24).map(|offset| anchor + offset).collect();
    let ids = tombstone_document_ids(&db, &tombstoned)
        .await
        .expect("tombstone the run");

    let mut tx = db.pool().begin().await.expect("begin");
    let refused = db.reserve_document_id_on(&mut tx).await;
    tx.rollback().await.expect("rollback");

    let error = refused.expect_err("a run of tombstoned values must be refused");
    assert!(
        error.to_string().contains("tombstoned"),
        "the refusal must name the tombstoned ids: {error}"
    );

    clear_tombstones(&db, &ids).await.expect("clear");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_bulk_document_tombstone_advances_the_generations_once_and_rolls_back() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");

    async fn seed_document(db: &Database, group_id: i64) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO context69.documents \
             (source_key, external_id, title, source_uri, updated_at_source, record_hash, \
              group_id, visibility) \
             VALUES ('p24-bulk', $1, 'bulk', 'urn:p24:bulk', now(), 'bulk-hash', $2, 'public') \
             RETURNING id",
        )
        .bind(format!("p24-bulk-{}", Uuid::new_v4()))
        .bind(group_id)
        .fetch_one(db.pool())
        .await
        .expect("seed the bulk document")
    }

    let first = seed_document(&db, group.id).await;
    let second = seed_document(&db, group.id).await;
    let missing = 700_000_000 + (Uuid::new_v4().as_u128() % 1_000) as i64;

    let epoch_before = registry_epoch(&db).await.expect("epoch before");
    let generation_before = search_generation(&db).await.expect("generation before");

    let mut tx = db.pool().begin().await.expect("begin");
    let inserted = db
        .insert_deleted_document_entities_in(&mut tx, &[first, second, missing])
        .await
        .expect("tombstone the document set");
    assert_eq!(
        inserted, 2,
        "only the existing documents are new tombstones"
    );
    tx.commit().await.expect("commit");

    let state = db.read_search_registry_state().await.expect("read state");
    assert!(state.deleted_document_ids.contains(&first));
    assert!(state.deleted_document_ids.contains(&second));
    assert_eq!(
        state.epoch,
        epoch_before + 1,
        "one bulk insertion advances the registry epoch exactly once"
    );
    assert_eq!(
        search_generation(&db).await.expect("generation"),
        generation_before + 1,
        "one bulk insertion invalidates the search cache exactly once"
    );

    let mut tx = db.pool().begin().await.expect("begin");
    let repeated = db
        .insert_deleted_document_entities_in(&mut tx, &[first, second])
        .await
        .expect("repeat the document set");
    let empty = db
        .insert_deleted_document_entities_in(&mut tx, &[])
        .await
        .expect("tombstone an empty set");
    tx.commit().await.expect("commit");
    assert_eq!(repeated, 0, "a repeated set inserts nothing");
    assert_eq!(empty, 0, "an empty set inserts nothing");
    assert_eq!(
        registry_epoch(&db).await.expect("epoch"),
        epoch_before + 1,
        "a repeated or empty set bumps no generation"
    );

    let rolled_back = seed_document(&db, group.id).await;
    let mut tx = db.pool().begin().await.expect("begin");
    assert_eq!(
        db.insert_deleted_document_entities_in(&mut tx, &[rolled_back])
            .await
            .expect("tombstone inside the rolled-back transaction"),
        1
    );
    tx.rollback().await.expect("rollback");
    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        !state.deleted_document_ids.contains(&rolled_back),
        "a rollback removes the tombstone with the caller transaction"
    );
    assert_eq!(
        state.epoch,
        epoch_before + 1,
        "a rolled-back tombstone bumps no generation"
    );

    for document_id in [first, second, rolled_back] {
        sqlx::query("DELETE FROM context69.documents WHERE id = $1")
            .bind(document_id)
            .execute(db.pool())
            .await
            .expect("delete the bulk document");
    }
    let ids = tombstone_ids(&db, &[], &[first, second])
        .await
        .expect("tombstone ids");
    clear_tombstones(&db, &ids).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_tombstone_table_survives_the_deletion_of_its_group() -> Result<()> {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file = FileFixture::create(&db, group.id)
        .await
        .expect("create file");

    file.remove(&db).await.expect("delete the file");
    let ids = tombstone_ids(&db, &[file.id], &[])
        .await
        .expect("tombstone ids");
    assert_eq!(ids.len(), 1, "the delete backstop wrote one tombstone");

    group.remove(&db).await.expect("remove the group");

    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        state.deleted_file_ids.contains(&file.id),
        "a tombstone must survive the deletion of its group"
    );

    clear_tombstones(&db, &ids).await.expect("clear");
    Ok(())
}

/// The recorded origin is immutable and part of the purge compare-and-set: a
/// later delete never rewrites it, a different target never stamps the
/// observation, and a repeated stamp of the same target inserts nothing.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_purge_stamp_requires_the_recorded_origin() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_id = Uuid::new_v4();

    let tombstone_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.library_deleted_entities \
         (file_id, group_id, target_endpoint, target_collection) \
         VALUES ($1, $2, 'http://origin.test:6334', 'origin-collection') RETURNING id",
    )
    .bind(file_id)
    .bind(group.id)
    .fetch_one(db.pool())
    .await
    .expect("insert a tombstone with a recorded origin");

    // A repeated delete of the same entity inserts nothing and cannot overwrite
    // the recorded origin with a later current target.
    let repeated: Option<i64> = sqlx::query_scalar(
        "INSERT INTO context69.library_deleted_entities \
         (file_id, group_id, target_endpoint, target_collection) \
         VALUES ($1, $2, 'http://other.test:6334', 'other-collection') \
         ON CONFLICT (file_id) WHERE file_id IS NOT NULL DO NOTHING RETURNING id",
    )
    .bind(file_id)
    .bind(group.id)
    .fetch_optional(db.pool())
    .await
    .expect("repeat the tombstone");
    assert!(repeated.is_none(), "a repeated delete inserts no row");

    let entity = db
        .read_deleted_entity(tombstone_id)
        .await
        .expect("read the tombstone")
        .expect("the tombstone exists");
    assert_eq!(
        entity.target_endpoint.as_deref(),
        Some("http://origin.test:6334")
    );
    assert_eq!(
        entity.target_collection.as_deref(),
        Some("origin-collection")
    );

    let mut tx = db.pool().begin().await.expect("begin");
    assert!(
        !db.mark_deleted_entity_purged_in(
            &mut tx,
            tombstone_id,
            Some("http://other.test:6334"),
            Some("other-collection"),
        )
        .await
        .expect("stamp with the wrong target"),
        "a different target cannot stamp the observation"
    );
    assert!(
        db.mark_deleted_entity_purged_in(
            &mut tx,
            tombstone_id,
            Some("http://origin.test:6334"),
            Some("origin-collection"),
        )
        .await
        .expect("stamp with the recorded target"),
        "the recorded target stamps the observation"
    );
    assert!(
        !db.mark_deleted_entity_purged_in(
            &mut tx,
            tombstone_id,
            Some("http://origin.test:6334"),
            Some("origin-collection"),
        )
        .await
        .expect("stamp a second time"),
        "a repeated stamp inserts nothing"
    );
    tx.commit().await.expect("commit");

    clear_tombstones(&db, &[tombstone_id]).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}

/// A pending/read representation never renders the private recorded endpoint,
/// including when that endpoint embeds credentials. Reporting keeps the rest
/// of the tombstone usable for operator verification.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_pending_tombstone_never_renders_its_credential_endpoint() {
    let db = connect_scratch().await;
    let group = GroupFixture::create(&db).await.expect("create group");
    let file_id = Uuid::new_v4();
    let endpoint = "http://operator:super-secret@origin.test:6334";

    let tombstone_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.library_deleted_entities \
         (file_id, group_id, target_endpoint, target_collection) \
         VALUES ($1, $2, $3, 'credential-collection') RETURNING id",
    )
    .bind(file_id)
    .bind(group.id)
    .bind(endpoint)
    .fetch_one(db.pool())
    .await
    .expect("insert a tombstone with a credential endpoint");

    let entity = db
        .read_deleted_entity(tombstone_id)
        .await
        .expect("read the tombstone")
        .expect("the tombstone exists");
    for rendering in [format!("{entity:?}"), format!("{entity:#?}")] {
        assert!(
            !rendering.contains("super-secret")
                && !rendering.contains("operator:")
                && !rendering.contains("origin.test")
                && !rendering.contains("http://"),
            "the debug rendering leaked the endpoint: {rendering}"
        );
        assert!(
            rendering.contains("credential-collection"),
            "the collection remains available for verification: {rendering}"
        );
    }

    let pending = db
        .list_pending_deleted_entities(1000)
        .await
        .expect("read the pending list");
    let listed = pending
        .iter()
        .find(|candidate| candidate.id == tombstone_id)
        .expect("the unknown-origin-safe pending row is reportable");
    assert!(
        !format!("{listed:?}").contains("super-secret"),
        "the pending representation must not render the credential"
    );

    let claimed = db
        .claim_pending_deleted_entities(1000)
        .await
        .expect("claim the actionable tombstones");
    let claimed_row = claimed
        .iter()
        .find(|candidate| candidate.id == tombstone_id)
        .expect("the known-origin row enters the attempt queue");
    assert!(
        !format!("{claimed_row:?}").contains("super-secret"),
        "the claimed representation must not render the credential"
    );

    clear_tombstones(&db, &[tombstone_id]).await.expect("clear");
    group.remove(&db).await.expect("remove group");
}
