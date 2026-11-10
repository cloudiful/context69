#[path = "library_ingest_switch/harness.rs"]
mod harness;

use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::domain::ChunkPayload;
use context69::library_store::{LibraryStore, NewLibraryFile, TaskWriteFence, begin_task_write};
use context69::services::library::{ReplacementSwitchDisposition, switch_replacement};
use context69_contracts::Visibility;
use harness::SwitchFixture;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn reserved_document_identity_reuses_version_and_rolls_back() {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .unwrap();
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .unwrap();
    let mut tx = db.pool().begin().await.unwrap();
    let key = format!("switch-{}", Uuid::new_v4());
    let group = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        key,
        "Switch",
        format!("test/{key}")
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let id = db.reserve_document_id_on(&mut tx).await.unwrap();
    let payload = ChunkPayload {
        chunk_id: Uuid::nil(),
        document_id: id,
        group_id: group,
        group_key: key.clone(),
        group_path: format!("test/{key}"),
        visibility: Visibility::Private,
        source_key: "library".into(),
        external_id: Uuid::new_v4().to_string(),
        title: "test".into(),
        summary: None,
        source_uri: "context69://test".into(),
        published_at: None,
        updated_at_source: chrono::Utc::now(),
        record_hash: "same".into(),
        chunk_index: 0,
        chunk_text: "content".into(),
        metadata_json: serde_json::json!({}),
        content_locale: "original".into(),
        source_locale: None,
        translation_provider: None,
    };
    let first = db
        .upsert_document_with_reserved_id_in(&mut tx, &payload, id)
        .await
        .unwrap();
    assert_eq!(first.document_id, id);
    assert!(first.changed);
    let second = db
        .upsert_document_with_reserved_id_in(&mut tx, &payload, id)
        .await
        .unwrap();
    assert_eq!(second.document_id, id);
    assert!(!second.changed);
    let count = sqlx::query_file_scalar!("tests/library_ingest_switch/version_count.sql", id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let original = Uuid::new_v4();
    db.insert_document_chunks_in(
        &mut tx,
        id,
        "same",
        &[context69::domain::DocumentChunk {
            id: original,
            document_id: id,
            chunk_index: 0,
            text: "content".into(),
            record_hash: "same".into(),
        }],
    )
    .await
    .unwrap();
    let current_version = Uuid::new_v4();
    let current_translation = Uuid::new_v4();
    let old_version = Uuid::new_v4();
    let old_translation = Uuid::new_v4();
    for (version, chunk, locale, hash) in [
        (current_version, current_translation, "en", "same"),
        (old_version, old_translation, "fr", "older"),
    ] {
        sqlx::query_file!(
            "tests/library_ingest_switch/seed_translation.sql",
            version,
            id,
            locale,
            hash
        )
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query_file!(
            "tests/library_ingest_switch/seed_translation_chunk.sql",
            chunk,
            version,
            id,
            locale
        )
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    let locked = db
        .lock_replacement_documents_on(&mut tx, &[id])
        .await
        .unwrap();
    assert_eq!(locked.len(), 1);
    assert_eq!(locked[0].record_hash, "same");
    assert_eq!(
        db.replacement_point_ids_on(&mut tx, id, "same", false)
            .await
            .unwrap(),
        vec![original]
    );
    // A same-hash reindex keeps the translation of the current hash and
    // retires only the translation that was made from an older hash.
    let same_hash = db
        .replacement_point_ids_on(&mut tx, id, "same", true)
        .await
        .unwrap();
    assert_eq!(same_hash.len(), 2);
    assert!(same_hash.contains(&original) && same_hash.contains(&old_translation));
    assert!(
        !same_hash.contains(&current_translation),
        "a translation of the new hash survives a same-hash reindex"
    );
    // A replacement that publishes another hash retires every stored
    // translation, including the one of an older version.
    let other_hash = db
        .replacement_point_ids_on(&mut tx, id, "updated", true)
        .await
        .unwrap();
    assert_eq!(other_hash.len(), 3);
    assert!(
        other_hash.contains(&original)
            && other_hash.contains(&current_translation)
            && other_hash.contains(&old_translation)
    );
    assert!(
        db.upsert_document_with_reserved_id_in(&mut tx, &payload, id + 1)
            .await
            .is_err()
    );
    let mut renamed = payload.clone();
    renamed.external_id = format!("updated-{}", Uuid::new_v4());
    renamed.record_hash = "updated".into();
    let updated = db
        .upsert_document_with_reserved_id_in(&mut tx, &renamed, id)
        .await
        .unwrap();
    assert_eq!(updated.document_id, id);
    assert!(updated.changed);
    let locked = db
        .lock_replacement_documents_on(&mut tx, &[id])
        .await
        .unwrap();
    assert_eq!(locked[0].external_id, renamed.external_id);
    assert_eq!(locked[0].record_hash, renamed.record_hash);
    let other_id = db.reserve_document_id_on(&mut tx).await.unwrap();
    let mut other = payload.clone();
    other.external_id = format!("other-{}", Uuid::new_v4());
    assert_eq!(
        db.upsert_document_with_reserved_id_in(&mut tx, &other, other_id)
            .await
            .unwrap()
            .document_id,
        other_id
    );
    let mut collision = renamed.clone();
    collision.external_id = other.external_id;
    assert!(
        db.upsert_document_with_reserved_id_in(&mut tx, &collision, id)
            .await
            .is_err()
    );
    let locked_after = db
        .lock_replacement_documents_on(&mut tx, &[id])
        .await
        .unwrap();
    assert_eq!(locked_after[0].external_id, renamed.external_id);
    assert_eq!(locked_after[0].record_hash, renamed.record_hash);
    assert_eq!(
        sqlx::query_file_scalar!("tests/library_ingest_switch/version_count.sql", id)
            .fetch_one(&mut *tx)
            .await
            .unwrap(),
        2
    );
    tx.rollback().await.unwrap();
    assert!(
        db.find_document_id_by_key(group, &payload.source_key, &payload.external_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn replacement_file_snapshot_and_attached_source_are_fenced() {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .unwrap();
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .unwrap();
    let user = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("switch-{}", Uuid::new_v4()),
        "Switch",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    let group = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("switch-{}", Uuid::new_v4()),
        "Switch",
        format!("test/switch-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    let task = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: task,
            user_id: user,
            group_id: Some(group),
            kind: "text_batch",
            group_path: Some("test/switch"),
            source_key: None,
            payloads: &[serde_json::json!({})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("switch-{}", Uuid::new_v4()),
        })
        .await
        .unwrap();
    let item = db
        .claim_items_fast(50)
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == items[0])
        .unwrap();
    let execution = item.execution_context().unwrap();
    let fence = TaskWriteFence::from_claimed_item(&item).unwrap();
    let store = LibraryStore::new(db.clone());
    let file = NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: None,
        filename: "switch.txt".into(),
        media_type: "text/plain".into(),
        size_bytes: 4,
        sha256: "0".repeat(64),
        storage_rel_path: format!("objects/{}", Uuid::new_v4()),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    store
        .create_file_in_project_fenced(fence, group, &file)
        .await
        .unwrap()
        .unwrap();
    assert!(
        db.set_task_item_file(task, item.id, &execution, file.id)
            .await
            .unwrap()
    );
    let object = Uuid::new_v4();
    let hash = "1".repeat(64);
    let object_key = format!("objects/{object}");
    sqlx::query_file!(
        "tests/library_ingest_switch/insert_object.sql",
        object,
        group,
        hash,
        8_i64,
        object_key
    )
    .execute(db.pool())
    .await
    .unwrap();
    let mut write = begin_task_write(&db, fence).await.unwrap();
    let snapshot = store
        .lock_replacement_file_on(write.connection(), file.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !store
            .switch_replacement_file_in(&mut write, &snapshot, Some(object), None)
            .await
            .unwrap()
    );
    write.rollback().await;
    assert!(
        db.set_task_item_input_storage_object(item.id, &execution, object, &serde_json::json!({}))
            .await
            .unwrap()
    );
    let mut stale = snapshot.clone();
    stale.folder_id = Some(Uuid::new_v4());
    let mut write = begin_task_write(&db, fence).await.unwrap();
    assert!(
        !store
            .switch_replacement_file_in(&mut write, &stale, Some(object), None)
            .await
            .unwrap()
    );
    assert!(
        store
            .switch_replacement_file_in(&mut write, &snapshot, Some(object), None)
            .await
            .unwrap()
    );
    write.rollback().await;
    let mut write = begin_task_write(&db, fence).await.unwrap();
    assert_eq!(
        store
            .lock_replacement_file_on(write.connection(), file.id)
            .await
            .unwrap()
            .unwrap(),
        snapshot
    );
    assert!(
        store
            .switch_replacement_file_in(&mut write, &snapshot, Some(object), None)
            .await
            .unwrap()
    );
    assert!(write.commit().await.unwrap());
    let mut write = begin_task_write(&db, fence).await.unwrap();
    let after = store
        .lock_replacement_file_on(write.connection(), file.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.storage_object_id, Some(object));
    assert_eq!(after.storage_rel_path, object_key);
    assert_eq!(after.sha256, hash);
    assert_eq!(after.size_bytes, 8);
    assert_eq!(
        store
            .get_file(file.id)
            .await
            .unwrap()
            .unwrap()
            .ingest_status,
        context69::contracts::LibraryIngestStatus::Succeeded
    );
    assert!(
        !store
            .switch_replacement_file_in(&mut write, &snapshot, Some(object), None)
            .await
            .unwrap()
    );
    write.rollback().await;
}

/// The switch moves the documents, the chunks, the links, the source, the
/// status, the registry entry and the durable run together, and it leaves a
/// vanished section and the results of the replaced document alone.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn replacement_switch_moves_every_layer_together() {
    let fixture = SwitchFixture::new().await;
    let kept = fixture.seed_section("kept", "old-hash").await;
    let vanished = fixture.seed_section("vanished", "vanished-hash").await;
    let stale_translation_version = Uuid::new_v4();
    let stale_translation = Uuid::new_v4();
    sqlx::query_file!(
        "tests/library_ingest_switch/seed_translation.sql",
        stale_translation_version,
        kept.document_id,
        "fr",
        "older-hash"
    )
    .execute(fixture.db.pool())
    .await
    .unwrap();
    sqlx::query_file!(
        "tests/library_ingest_switch/seed_translation_chunk.sql",
        stale_translation,
        stale_translation_version,
        kept.document_id,
        "fr"
    )
    .execute(fixture.db.pool())
    .await
    .unwrap();
    sqlx::query_file!(
        "tests/library_ingest_switch/insert_document_result.sql",
        kept.document_id,
        fixture.user,
        "switch",
        "summary",
        "test",
        "old-hash",
        "result-1",
        serde_json::json!({"answer": "kept"})
    )
    .execute(fixture.db.pool())
    .await
    .unwrap();
    let staged = fixture.attach_staged_object().await;
    let (run_id, generation) = fixture.create_run().await;
    let chunk = Uuid::new_v4();
    let added_chunk = Uuid::new_v4();
    let reserved = fixture.reserve_document_id().await;
    fixture
        .admit(
            run_id,
            generation,
            vec![kept.document_id, reserved],
            vec![chunk, added_chunk],
            2,
        )
        .await;
    let baseline = fixture.baseline().await;
    let mut plan = fixture.plan_section(
        run_id,
        baseline,
        &kept,
        "kept",
        "new-hash",
        chunk,
        Some(staged),
    );
    fixture.add_new_section(
        &mut plan,
        reserved,
        "added",
        "added-external",
        "added-hash",
        added_chunk,
    );

    let outcome = switch_replacement(&fixture.db, &fixture.store, fixture.fence, &plan)
        .await
        .unwrap();
    assert_eq!(outcome.disposition, ReplacementSwitchDisposition::Switched);
    assert_eq!(outcome.document_ids, vec![kept.document_id, reserved]);
    let mut retired = outcome.retired_point_ids.clone();
    retired.sort();
    let mut expected_retired = vec![kept.chunk_id, stale_translation];
    expected_retired.sort();
    assert_eq!(retired, expected_retired);

    let live_chunks = fixture
        .db
        .list_chunk_ids_for_document(kept.document_id)
        .await
        .unwrap();
    assert!(
        live_chunks.contains(&chunk),
        "the replacement chunk is live"
    );
    assert!(
        !live_chunks.contains(&kept.chunk_id),
        "the retired original chunk is gone from the live rows"
    );
    assert!(
        live_chunks.contains(&stale_translation),
        "the stale translation row stays as history"
    );
    assert_eq!(
        fixture
            .db
            .list_chunk_ids_for_document(vanished.document_id)
            .await
            .unwrap(),
        vec![vanished.chunk_id]
    );
    assert_eq!(
        fixture
            .db
            .find_document_id_by_key(fixture.group, "library", "added-external")
            .await
            .unwrap(),
        Some(reserved)
    );
    assert_eq!(
        fixture
            .db
            .list_chunk_ids_for_document(reserved)
            .await
            .unwrap(),
        vec![added_chunk]
    );
    let links = fixture
        .store
        .list_file_documents(fixture.file_id)
        .await
        .unwrap();
    assert_eq!(links.len(), 3);
    assert!(
        links
            .iter()
            .any(|link| link.document_id == vanished.document_id)
    );
    assert!(links.iter().any(|link| link.document_id == reserved));

    let entry = fixture.registry_entry(run_id).await;
    assert_eq!(entry.phase, "switched");
    assert!(entry.staged_ids.is_empty());
    let mut expected_retired = vec![kept.chunk_id, stale_translation];
    expected_retired.sort();
    let mut stored_retired = entry.retired_ids.clone();
    stored_retired.sort();
    assert_eq!(stored_retired, expected_retired);
    let origin = sqlx::query_file!("tests/sql/vector_registry/read_retired_origin.sql", run_id)
        .fetch_one(fixture.db.pool())
        .await
        .unwrap();
    assert_eq!(origin.retired_original_ids, vec![kept.chunk_id]);
    assert_eq!(origin.reserved_ids, 2);

    let run = fixture
        .db
        .read_library_ingest_run(fixture.item_id)
        .await
        .unwrap()
        .unwrap();
    assert!(run.resolved_at.is_some());

    let file = sqlx::query_file_as!(
        harness::FileSourceRow,
        "tests/library_ingest_switch/file_source.sql",
        fixture.file_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(file.ingest_status, "succeeded");
    assert_eq!(file.storage_object_id, Some(staged));
    assert_eq!(file.storage_rel_path, format!("objects/{staged}"));
    assert_eq!(file.sha256, "1".repeat(64));
    assert_eq!(file.size_bytes, 8);

    let results = sqlx::query_file_scalar!(
        "tests/library_ingest_switch/document_result_count.sql",
        kept.document_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(results, 1);

    let versions = sqlx::query_file_scalar!(
        "tests/library_ingest_switch/version_count.sql",
        kept.document_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(versions, 2);

    let intents = sqlx::query_file_scalar!(
        "tests/library_ingest_switch/cleanup_intent_count.sql",
        fixture.old_object
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(intents, 1);
}

/// A document that moved on after the replacement was prepared refuses the
/// switch, and the refusal leaves every layer as it was.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_baseline_conflict_rolls_every_layer_back() {
    let fixture = SwitchFixture::new().await;
    let kept = fixture.seed_section("kept", "old-hash").await;
    let staged = fixture.attach_staged_object().await;
    let (run_id, generation) = fixture.create_run().await;
    let chunk = Uuid::new_v4();
    fixture
        .admit(run_id, generation, vec![kept.document_id], vec![chunk], 1)
        .await;
    let baseline = fixture.baseline().await;
    let plan = fixture.plan_section(
        run_id,
        baseline,
        &kept,
        "kept",
        "new-hash",
        chunk,
        Some(staged),
    );

    let mut moved_on = fixture.payload(&kept.external_id, "moved-on-hash", Uuid::new_v4());
    moved_on.document_id = kept.document_id;
    fixture.db.upsert_document(&moved_on).await.unwrap();

    assert!(
        switch_replacement(&fixture.db, &fixture.store, fixture.fence, &plan)
            .await
            .is_err()
    );

    assert_eq!(
        fixture
            .db
            .list_chunk_ids_for_document(kept.document_id)
            .await
            .unwrap(),
        vec![kept.chunk_id]
    );
    let file = sqlx::query_file_as!(
        harness::FileSourceRow,
        "tests/library_ingest_switch/file_source.sql",
        fixture.file_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_ne!(file.ingest_status, "succeeded");
    assert_eq!(file.storage_object_id, Some(fixture.old_object));
    assert_eq!(fixture.registry_entry(run_id).await.phase, "staging");
    let run = fixture
        .db
        .read_library_ingest_run(fixture.item_id)
        .await
        .unwrap()
        .unwrap();
    assert!(run.resolved_at.is_none());
}

/// A registry entry that does not hold exactly the prepared chunks refuses the
/// switch, so a switch can never publish a point set the admission did not
/// register.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_registered_staged_set_must_match_the_prepared_chunks() {
    let fixture = SwitchFixture::new().await;
    let kept = fixture.seed_section("kept", "old-hash").await;
    let staged = fixture.attach_staged_object().await;
    let (run_id, generation) = fixture.create_run().await;
    let chunk = Uuid::new_v4();
    fixture
        .admit(
            run_id,
            generation,
            vec![kept.document_id],
            vec![Uuid::new_v4()],
            1,
        )
        .await;
    let baseline = fixture.baseline().await;
    let plan = fixture.plan_section(
        run_id,
        baseline,
        &kept,
        "kept",
        "new-hash",
        chunk,
        Some(staged),
    );

    assert!(
        switch_replacement(&fixture.db, &fixture.store, fixture.fence, &plan)
            .await
            .is_err()
    );

    assert_eq!(
        fixture
            .db
            .list_chunk_ids_for_document(kept.document_id)
            .await
            .unwrap(),
        vec![kept.chunk_id]
    );
    assert_eq!(fixture.registry_entry(run_id).await.phase, "staging");
    let run = fixture
        .db
        .read_library_ingest_run(fixture.item_id)
        .await
        .unwrap()
        .unwrap();
    assert!(run.resolved_at.is_none());
}

/// A replay of a resolved run reports that it was already applied and never
/// rewrites a state that moved on after the first switch.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_resolved_run_replays_without_rewriting_a_later_state() {
    let fixture = SwitchFixture::new().await;
    let kept = fixture.seed_section("kept", "old-hash").await;
    let staged = fixture.attach_staged_object().await;
    let (run_id, generation) = fixture.create_run().await;
    let chunk = Uuid::new_v4();
    fixture
        .admit(run_id, generation, vec![kept.document_id], vec![chunk], 1)
        .await;
    let baseline = fixture.baseline().await;
    let plan = fixture.plan_section(
        run_id,
        baseline,
        &kept,
        "kept",
        "new-hash",
        chunk,
        Some(staged),
    );
    switch_replacement(&fixture.db, &fixture.store, fixture.fence, &plan)
        .await
        .unwrap();

    let newer_chunk = Uuid::new_v4();
    let mut newer = fixture.payload(&kept.external_id, "newer-hash", newer_chunk);
    newer.document_id = kept.document_id;
    fixture.db.upsert_document(&newer).await.unwrap();
    fixture
        .db
        .delete_document_chunks(kept.document_id)
        .await
        .unwrap();
    fixture
        .db
        .insert_document_chunks(
            kept.document_id,
            "newer-hash",
            &[context69::domain::DocumentChunk {
                id: newer_chunk,
                document_id: kept.document_id,
                chunk_index: 0,
                text: "newer".into(),
                record_hash: "newer-hash".into(),
            }],
        )
        .await
        .unwrap();

    let replay = switch_replacement(&fixture.db, &fixture.store, fixture.fence, &plan)
        .await
        .unwrap();
    assert_eq!(
        replay.disposition,
        ReplacementSwitchDisposition::AlreadyApplied
    );
    assert_eq!(
        fixture
            .db
            .list_chunk_ids_for_document(kept.document_id)
            .await
            .unwrap(),
        vec![newer_chunk]
    );
    let versions = sqlx::query_file_scalar!(
        "tests/library_ingest_switch/version_count.sql",
        kept.document_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(versions, 3);
}

/// A grant that the task item no longer holds refuses the switch before any
/// statement of the replacement runs.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_lost_grant_refuses_the_switch() {
    let fixture = SwitchFixture::new().await;
    let kept = fixture.seed_section("kept", "old-hash").await;
    let staged = fixture.attach_staged_object().await;
    let (run_id, generation) = fixture.create_run().await;
    let chunk = Uuid::new_v4();
    fixture
        .admit(run_id, generation, vec![kept.document_id], vec![chunk], 1)
        .await;
    let baseline = fixture.baseline().await;
    let plan = fixture.plan_section(
        run_id,
        baseline,
        &kept,
        "kept",
        "new-hash",
        chunk,
        Some(staged),
    );

    sqlx::query_file!(
        "tests/library_ingest_switch/revoke_lease.sql",
        Uuid::new_v4(),
        fixture.item_id
    )
    .execute(fixture.db.pool())
    .await
    .unwrap();

    assert!(
        switch_replacement(&fixture.db, &fixture.store, fixture.fence, &plan)
            .await
            .is_err()
    );

    assert_eq!(
        fixture
            .db
            .list_chunk_ids_for_document(kept.document_id)
            .await
            .unwrap(),
        vec![kept.chunk_id]
    );
    let file = sqlx::query_file_as!(
        harness::FileSourceRow,
        "tests/library_ingest_switch/file_source.sql",
        fixture.file_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_ne!(file.ingest_status, "succeeded");
    assert_eq!(fixture.registry_entry(run_id).await.phase, "staging");
}

/// A staged source object that the task item does not reference refuses the
/// switch, and the source the file already has is never released twice.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_staged_source_outside_the_task_item_refuses_the_switch() {
    let fixture = SwitchFixture::new().await;
    let kept = fixture.seed_section("kept", "old-hash").await;
    let foreign = fixture.create_object().await;
    let (run_id, generation) = fixture.create_run().await;
    let chunk = Uuid::new_v4();
    fixture
        .admit(run_id, generation, vec![kept.document_id], vec![chunk], 1)
        .await;
    let baseline = fixture.baseline().await;
    let plan = fixture.plan_section(
        run_id,
        baseline.clone(),
        &kept,
        "kept",
        "new-hash",
        chunk,
        Some(foreign),
    );

    assert!(
        switch_replacement(&fixture.db, &fixture.store, fixture.fence, &plan)
            .await
            .is_err()
    );
    let file = sqlx::query_file_as!(
        harness::FileSourceRow,
        "tests/library_ingest_switch/file_source.sql",
        fixture.file_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_ne!(file.ingest_status, "succeeded");
    assert_eq!(file.storage_object_id, Some(fixture.old_object));

    fixture.attach_object(fixture.old_object).await;
    let same = fixture.plan_section(
        run_id,
        baseline,
        &kept,
        "kept",
        "new-hash",
        chunk,
        Some(fixture.old_object),
    );
    let outcome = switch_replacement(&fixture.db, &fixture.store, fixture.fence, &same)
        .await
        .unwrap();
    assert_eq!(outcome.disposition, ReplacementSwitchDisposition::Switched);
    let file = sqlx::query_file_as!(
        harness::FileSourceRow,
        "tests/library_ingest_switch/file_source.sql",
        fixture.file_id
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(file.ingest_status, "succeeded");
    assert_eq!(file.storage_object_id, Some(fixture.old_object));
    let intents = sqlx::query_file_scalar!(
        "tests/library_ingest_switch/cleanup_intent_count.sql",
        fixture.old_object
    )
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(intents, 0);
}
