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
        db.mark_deleted_entity_purged_in(&mut tx, ids[0])
            .await
            .expect("mark purged"),
        "the first purge stamps the observation"
    );
    assert!(
        !db.mark_deleted_entity_purged_in(&mut tx, ids[0])
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
