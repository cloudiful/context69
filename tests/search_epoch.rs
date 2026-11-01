//! Search registry epoch persistence tests (issue #1138 P3, lifecycle coverage
//! extended by P6, lifecycle semantics corrected by P9).
//!
//! They pin the properties the future safe reingest design depends on: the
//! registry state is one consistent statement (epoch and entries always belong
//! to the same snapshot, empty registry included), and every mutation is a
//! transactional conditional write that bumps the epoch exactly once when it
//! commits. Rejected or rolled-back writes leave both the data and the epoch
//! untouched.
//!
//! The lifecycle owns the id arrays of an entry: a switch releases the staged
//! ids as the live version and stores the old live ids as the retired set, an
//! abandoned run turns its staged ids into the retired set, and a resolved
//! entry keeps that set as the tombstone of the ids that are no longer live.
//!
//! These tests run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and supplies it as
//! `CONTEXT69_TEST_DATABASE_URL`; they are skipped (reported `ignored`) by a
//! plain `cargo test --workspace` and are reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.

use context69::db::{Database, RegistryQuotaOverride};
use context69_search::{SearchRegistryEntry, SearchRegistryState};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires configured reusable PostgreSQL"]
async fn bounded_admission_serializes_and_preserves_reservations() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;
    let before = db.read_search_registry_epoch().await.unwrap();
    let file = Uuid::new_v4();
    let caps = RegistryQuotaOverride::exact([3, 3, 3]);
    let attempts: Vec<_> = (0..2)
        .map(|_| {
            let db = db.clone();
            tokio::spawn(async move {
                let mut entry = registry_entry(Uuid::new_v4(), 1);
                entry.file_id = Some(file);
                entry.retired_ids.clear();
                let mut tx = db.pool().begin().await.unwrap();
                let admitted = db
                    .admit_search_registry_entry_in(&entry, 1, caps, &mut tx)
                    .await
                    .unwrap();
                tx.commit().await.unwrap();
                (entry, admitted)
            })
        })
        .collect();
    let outcomes = futures::future::join_all(attempts)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.iter().filter(|(_, epoch)| epoch.is_some()).count(),
        1
    );
    let entry = &outcomes
        .iter()
        .find(|(_, epoch)| epoch.is_some())
        .unwrap()
        .0;
    assert_eq!(db.read_search_registry_epoch().await.unwrap(), before + 1);
    let mut tx = db.pool().begin().await.unwrap();
    let oversized = vec![Uuid::new_v4(); 4];
    assert_eq!(
        db.transition_search_registry_entry_in(
            entry.run_id,
            "staging",
            "switched",
            Some(&oversized),
            &mut tx
        )
        .await
        .unwrap(),
        None
    );
    tx.commit().await.unwrap();
    assert_eq!(db.read_search_registry_epoch().await.unwrap(), before + 1);
    let retired = vec![Uuid::new_v4()];
    assert!(
        transition_entry(&db, entry.run_id, "staging", "switched", Some(&retired))
            .await
            .is_some()
    );
    let mut next = registry_entry(Uuid::new_v4(), 2);
    next.file_id = Some(file);
    next.retired_ids.clear();
    let mut tx = db.pool().begin().await.unwrap();
    assert!(
        db.admit_search_registry_entry_in(&next, 0, caps, &mut tx)
            .await
            .unwrap()
            .is_some()
    );
    tx.rollback().await.unwrap();
    let state = db.read_search_registry_state().await.unwrap();
    assert_eq!(state.epoch, before + 2);
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].retired_ids, retired);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn bounded_admission_refuses_invalid_and_overlapping_entries() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;
    let mut base = registry_entry(Uuid::new_v4(), 1);
    base.retired_ids.clear();
    let caps = RegistryQuotaOverride::exact([10, 10, 10]);
    let mut tx = db.pool().begin().await.unwrap();
    db.admit_search_registry_entry_in(&base, 1, caps, &mut tx)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    let before = db.read_search_registry_state().await.unwrap();
    let mut candidate = base.clone();
    candidate.run_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.unwrap();
    assert_eq!(
        db.admit_search_registry_entry_in(&candidate, 0, caps, &mut tx)
            .await
            .unwrap(),
        None
    );
    candidate.staged_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
    assert_eq!(
        db.admit_search_registry_entry_in(
            &candidate,
            0,
            RegistryQuotaOverride::exact([4, 10, 10]),
            &mut tx
        )
        .await
        .unwrap(),
        None
    );
    assert_eq!(
        db.admit_search_registry_entry_in(
            &candidate,
            0,
            RegistryQuotaOverride::exact([10, 4, 10]),
            &mut tx
        )
        .await
        .unwrap(),
        None
    );
    assert_eq!(
        db.admit_search_registry_entry_in(
            &candidate,
            0,
            RegistryQuotaOverride::exact([10, 10, 1]),
            &mut tx
        )
        .await
        .unwrap(),
        None
    );
    candidate.staged_ids[1] = candidate.staged_ids[0];
    assert!(
        db.admit_search_registry_entry_in(&candidate, 0, caps, &mut tx)
            .await
            .is_err()
    );
    candidate.staged_ids.clear();
    assert!(
        db.admit_search_registry_entry_in(&candidate, 0, caps, &mut tx)
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
    let after = db.read_search_registry_state().await.unwrap();
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(after.entries, before.entries);
    transition_entry(&db, base.run_id, "staging", "retired", None)
        .await
        .unwrap();
    let mut tx = db.pool().begin().await.unwrap();
    candidate.staged_ids = base.staged_ids.clone();
    assert_eq!(
        db.admit_search_registry_entry_in(&candidate, 0, caps, &mut tx)
            .await
            .unwrap(),
        None
    );
    tx.commit().await.unwrap();
    candidate.staged_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
    let mut tx = db.pool().begin().await.unwrap();
    assert!(
        db.admit_search_registry_entry_in(
            &candidate,
            0,
            RegistryQuotaOverride::exact([4, 4, 4]),
            &mut tx
        )
        .await
        .unwrap()
        .is_some()
    );
    tx.commit().await.unwrap();
    let state = db.read_search_registry_state().await.unwrap();
    let tombstone = state
        .entries
        .iter()
        .find(|entry| entry.run_id == base.run_id)
        .unwrap();
    assert_eq!(tombstone.retired_ids, base.staged_ids);
    assert!(tombstone.staged_ids.is_empty());
}

async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

fn registry_entry(run_id: Uuid, generation: i64) -> SearchRegistryEntry {
    SearchRegistryEntry {
        run_id,
        file_id: Some(Uuid::new_v4()),
        document_ids: vec![1, 2, 3],
        staged_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
        retired_ids: vec![Uuid::new_v4()],
        baseline_record_hash: Some("baseline".to_string()),
        staged_record_hash: Some("staged".to_string()),
        generation,
        phase: "staging".to_string(),
    }
}

/// Clear the entries this test target owns (it is the only writer of the
/// registry table), so the empty-registry shape is observable on the shared
/// scratch database.
async fn clear_registry_entries(db: &Database) {
    sqlx::query("DELETE FROM context69.search_registry_entries")
        .execute(db.pool())
        .await
        .expect("clear registry entries");
}

async fn register_entry(db: &Database, entry: &SearchRegistryEntry) -> i64 {
    let mut tx = db.pool().begin().await.expect("begin registration");
    let epoch = db
        .register_search_registry_entry_with_epoch_in(entry, &mut tx)
        .await
        .expect("register entry and bump");
    tx.commit().await.expect("commit registration");
    epoch
}

async fn transition_entry(
    db: &Database,
    run_id: Uuid,
    expected_phase: &str,
    next_phase: &str,
    retired_ids: Option<&[Uuid]>,
) -> Option<i64> {
    let mut tx = db.pool().begin().await.expect("begin transition");
    let epoch = db
        .transition_search_registry_entry_in(
            run_id,
            expected_phase,
            next_phase,
            retired_ids,
            &mut tx,
        )
        .await
        .expect("conditional transition");
    tx.commit().await.expect("commit transition");
    epoch
}

async fn delete_entry(db: &Database, run_id: Uuid, expected_phase: &str) -> Option<i64> {
    let mut tx = db.pool().begin().await.expect("begin delete");
    let epoch = db
        .delete_search_registry_entry_in(run_id, expected_phase, &mut tx)
        .await
        .expect("conditional delete");
    tx.commit().await.expect("commit delete");
    epoch
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_empty_registry_reads_the_epoch_with_no_entries() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let state = db.read_search_registry_state().await.expect("read state");
    let epoch = db.read_search_registry_epoch().await.expect("read epoch");
    assert_eq!(
        state.epoch, epoch,
        "the statement and the scalar must agree"
    );
    assert!(
        state.entries.is_empty(),
        "an empty registry is the epoch plus no entries"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_entry_and_its_epoch_bump_commit_and_roll_back_together() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let before = db.read_search_registry_epoch().await.expect("epoch before");

    // Committed shape: the entry and the bump commit as one transaction and
    // the snapshot observes both.
    let run_id = Uuid::new_v4();
    let entry = registry_entry(run_id, before);
    let registered_epoch = register_entry(&db, &entry).await;
    assert_eq!(registered_epoch, before + 1);

    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(state.epoch, before + 1);
    assert_eq!(
        state.entries.iter().find(|stored| stored.run_id == run_id),
        Some(&entry),
        "the committed entry keeps every supplied field"
    );

    // A duplicate run id fails before the bump, so rolling back the failed
    // insert cannot change the epoch.
    let mut tx = db.pool().begin().await.expect("begin duplicate");
    let duplicate = db
        .register_search_registry_entry_with_epoch_in(&entry, &mut tx)
        .await;
    assert!(duplicate.is_err(), "a duplicate run id fails the insert");
    tx.rollback().await.expect("roll back duplicate");
    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(state.epoch, before + 1, "the failed insert never bumps");
    assert_eq!(
        state
            .entries
            .iter()
            .filter(|stored| stored.run_id == run_id)
            .count(),
        1,
        "the original entry is unchanged"
    );

    // Rolled-back shape: the same conditional transition and bump roll back
    // together and the snapshot observes neither.
    let after_commit = db.read_search_registry_epoch().await.expect("epoch after");
    let retired_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
    let mut tx = db.pool().begin().await.expect("begin rolled-back run");
    let transitioned = db
        .transition_search_registry_entry_in(
            run_id,
            "staging",
            "switched",
            Some(&retired_ids),
            &mut tx,
        )
        .await
        .expect("transition rolled-back run")
        .expect("the staging entry matches");
    assert_eq!(transitioned, after_commit + 1);
    tx.rollback().await.expect("roll back registry mutation");

    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(state.epoch, after_commit, "the epoch bump rolled back");
    assert_eq!(
        state.entries.iter().find(|stored| stored.run_id == run_id),
        Some(&entry),
        "the rolled-back phase and arrays are untouched"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_bumps_keep_the_epoch_monotonic() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;
    let before = db.read_search_registry_epoch().await.expect("epoch before");

    let writers: Vec<_> = (0..10)
        .map(|_| {
            let db = db.clone();
            tokio::spawn(async move {
                let mut tx = db.pool().begin().await.expect("begin bump");
                let epoch = db
                    .bump_search_registry_epoch_in(&mut tx)
                    .await
                    .expect("bump");
                tx.commit().await.expect("commit bump");
                epoch
            })
        })
        .collect();
    let observed: Vec<i64> = futures::future::join_all(writers)
        .await
        .into_iter()
        .map(|joined| joined.expect("bump task"))
        .collect();

    let after = db.read_search_registry_epoch().await.expect("epoch after");
    assert_eq!(
        after,
        before + 10,
        "every concurrent bump must land: no lost update"
    );
    let mut sorted = observed.clone();
    sorted.sort_unstable();
    assert!(
        sorted.windows(2).all(|pair| pair[0] < pair[1]),
        "each writer observed a distinct, monotonically increasing epoch: {observed:?}"
    );

    // The same compare-and-set rule decides concurrent phase transitions: a
    // stale writer that also thinks the entry is still staging loses.
    let run_id = Uuid::new_v4();
    register_entry(&db, &registry_entry(run_id, after)).await;
    let before_switch = db
        .read_search_registry_epoch()
        .await
        .expect("epoch before switch");
    let attempts: Vec<_> = (0..4)
        .map(|_| {
            let db = db.clone();
            tokio::spawn(async move {
                let retired_ids = vec![Uuid::new_v4()];
                let mut tx = db.pool().begin().await.expect("begin attempt");
                let epoch = db
                    .transition_search_registry_entry_in(
                        run_id,
                        "staging",
                        "switched",
                        Some(&retired_ids),
                        &mut tx,
                    )
                    .await
                    .expect("conditional transition");
                tx.commit().await.expect("commit attempt");
                (epoch, retired_ids)
            })
        })
        .collect();
    let mut winners = Vec::new();
    for outcome in futures::future::join_all(attempts).await {
        let (epoch, retired_ids) = outcome.expect("attempt task");
        if let Some(epoch) = epoch {
            winners.push((epoch, retired_ids));
        }
    }
    assert_eq!(winners.len(), 1, "only one same-phase attempt may win");

    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(
        state.epoch,
        before_switch + 1,
        "the single winner bumps the epoch exactly once"
    );
    assert_eq!(winners[0].0, before_switch + 1);
    let stored = state
        .entries
        .iter()
        .find(|stored| stored.run_id == run_id)
        .expect("the entry is retained");
    assert_eq!(stored.phase, "switched");
    assert_eq!(
        stored.retired_ids, winners[0].1,
        "the winning transition's retired ids are the stored ones"
    );
    assert!(
        stored.staged_ids.is_empty(),
        "the winning switch releases the staged ids as the live version"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_registry_snapshot_pairs_the_epoch_with_its_entries() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let entry = SearchRegistryEntry {
        run_id: Uuid::new_v4(),
        file_id: Some(Uuid::new_v4()),
        document_ids: vec![11, 22, 33, 44],
        staged_ids: vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
        retired_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
        baseline_record_hash: Some("baseline-hash".to_string()),
        staged_record_hash: Some("staged-hash".to_string()),
        generation: 42,
        phase: "staging".to_string(),
    };
    let registered_epoch = register_entry(&db, &entry).await;

    let state: SearchRegistryState = db.read_search_registry_state().await.expect("read state");
    assert_eq!(state.epoch, registered_epoch);
    let stored = state
        .entries
        .iter()
        .find(|stored| stored.run_id == entry.run_id)
        .expect("the new entry is in the same snapshot as the bumped epoch");
    assert_eq!(
        stored, &entry,
        "the snapshot persists every supplied array, hash and generation"
    );

    // Switching to `switched` stores the retired set and releases the staged
    // ids, because the staged ids are the live version from now on.
    let retired_ids = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let switched_epoch =
        transition_entry(&db, entry.run_id, "staging", "switched", Some(&retired_ids))
            .await
            .expect("the staging entry transitions");
    assert_eq!(switched_epoch, registered_epoch + 1);
    let state = db.read_search_registry_state().await.expect("read state");
    let stored = state
        .entries
        .iter()
        .find(|stored| stored.run_id == entry.run_id)
        .expect("switched entry is visible");
    assert_eq!(stored.phase, "switched");
    assert_eq!(
        stored.retired_ids, retired_ids,
        "the switch stores the retired set"
    );
    assert!(
        stored.staged_ids.is_empty(),
        "the switch releases the staged ids as the live version"
    );
    assert_eq!(stored.generation, entry.generation);

    // Retiring keeps the switched entry's retired set when none is supplied.
    let retired_epoch = transition_entry(&db, entry.run_id, "switched", "retired", None)
        .await
        .expect("the switched entry retires");
    assert_eq!(retired_epoch, switched_epoch + 1);
    let state = db.read_search_registry_state().await.expect("read state");
    let stored = state
        .entries
        .iter()
        .find(|stored| stored.run_id == entry.run_id)
        .expect("retired entry is visible");
    assert_eq!(stored.phase, "retired");
    assert_eq!(stored.retired_ids, retired_ids);

    // A stale or missing conditional transition/removal changes nothing.
    let epoch_before_rejections = retired_epoch;
    assert_eq!(
        transition_entry(&db, entry.run_id, "staging", "switched", Some(&retired_ids)).await,
        None,
        "a stale expected phase matches no row"
    );
    assert_eq!(
        transition_entry(
            &db,
            Uuid::new_v4(),
            "staging",
            "switched",
            Some(&retired_ids)
        )
        .await,
        None,
        "a missing run id matches no row"
    );
    assert_eq!(
        delete_entry(&db, entry.run_id, "staging").await,
        None,
        "a stale conditional removal matches no row"
    );
    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(
        state.epoch, epoch_before_rejections,
        "rejected lifecycle operations never bump the epoch"
    );
    assert!(
        state
            .entries
            .iter()
            .any(|stored| stored.run_id == entry.run_id),
        "rejected lifecycle operations retain the entry"
    );

    // A resolved entry is the tombstone of the ids that are no longer live, so
    // the lifecycle never removes it.
    let mut tx = db.pool().begin().await.expect("begin refused removal");
    let removal = db
        .delete_search_registry_entry_in(entry.run_id, "retired", &mut tx)
        .await;
    assert!(
        removal.is_err(),
        "a resolved entry is never removed by the lifecycle"
    );
    tx.rollback().await.expect("roll back refused removal");
    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(
        state.epoch, epoch_before_rejections,
        "a refused removal never bumps the epoch"
    );
    assert!(
        state
            .entries
            .iter()
            .any(|stored| stored.run_id == entry.run_id),
        "a refused removal retains the tombstone"
    );
}

/// The deleted entity sets belong to the same statement as the epoch and the
/// entries (issue #1138 P24), so a search attempt can never pair an epoch with
/// a different set of deleted files or documents.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_registry_snapshot_pairs_the_epoch_with_the_deleted_entity_sets() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let deleted_file = Uuid::new_v4();
    let deleted_document = 700_000_000 + (Uuid::new_v4().as_u128() % 1_000) as i64;

    let before = db.read_search_registry_epoch().await.expect("epoch before");
    let mut tx = db.pool().begin().await.expect("begin tombstone");
    let file_inserted = db
        .insert_deleted_file_entity_in(&mut tx, deleted_file, None)
        .await
        .expect("tombstone the file");
    assert!(file_inserted, "the file tombstone is a new insertion");
    let document_inserted = db
        .insert_deleted_document_entity_in(&mut tx, deleted_document, None)
        .await
        .expect("tombstone the document");
    assert!(
        document_inserted,
        "the document tombstone is a new insertion"
    );
    tx.commit().await.expect("commit tombstones");

    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(
        state.epoch,
        before + 2,
        "each new tombstone advances the epoch once, in the caller's transaction"
    );
    assert!(
        state.deleted_file_ids.contains(&deleted_file),
        "the deleted file is part of the epoch's snapshot"
    );
    assert!(
        state.deleted_document_ids.contains(&deleted_document),
        "the deleted document is part of the epoch's snapshot"
    );
    let exclusions = state.search_exclusions();
    assert!(
        exclusions.file_ids.contains(&deleted_file)
            && exclusions.document_ids.contains(&deleted_document),
        "the attempt exclusion set carries the same entity ids"
    );

    // A rolled-back tombstone leaves the snapshot exactly as it was.
    let epoch_after_commit = state.epoch;
    let rolled_back = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin rolled-back tombstone");
    assert!(
        db.insert_deleted_file_entity_in(&mut tx, rolled_back, None)
            .await
            .expect("tombstone")
    );
    tx.rollback().await.expect("roll back tombstone");
    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(
        state.epoch, epoch_after_commit,
        "a rolled-back tombstone never advances the epoch"
    );
    assert!(
        !state.deleted_file_ids.contains(&rolled_back),
        "a rolled-back tombstone never enters the snapshot"
    );

    sqlx::query(
        "DELETE FROM context69.library_deleted_entities WHERE file_id = ANY($1) \
         OR document_id = ANY($2)",
    )
    .bind(&[deleted_file, rolled_back][..])
    .bind(&[deleted_document][..])
    .execute(db.pool())
    .await
    .expect("clear the tombstones");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_switch_releases_the_new_live_ids_and_keeps_the_old_ones_excluded() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let entry = registry_entry(Uuid::new_v4(), 7);
    let staged_ids = entry.staged_ids.clone();
    register_entry(&db, &entry).await;

    let retired_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
    transition_entry(&db, entry.run_id, "staging", "switched", Some(&retired_ids))
        .await
        .expect("the staging entry switches");

    let state = db.read_search_registry_state().await.expect("read state");
    let stored = state
        .entries
        .iter()
        .find(|stored| stored.run_id == entry.run_id)
        .expect("the switched entry is visible");
    assert_eq!(stored.phase, "switched");
    assert_eq!(stored.retired_ids, retired_ids);
    assert!(
        stored.staged_ids.is_empty(),
        "the staged ids are the live version and must not stay excluded"
    );

    let exclusions = state.search_point_exclusions();
    assert_eq!(
        exclusions, retired_ids,
        "only the retired ids stay excluded after the switch"
    );
    assert!(
        !exclusions.contains(&staged_ids[0]),
        "the new live version must be searchable again"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_abandoned_run_moves_its_staged_ids_into_the_tombstone() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let entry = registry_entry(Uuid::new_v4(), 8);
    let staged_ids = entry.staged_ids.clone();
    register_entry(&db, &entry).await;

    transition_entry(&db, entry.run_id, "staging", "retired", None)
        .await
        .expect("the staging run is abandoned");

    let state = db.read_search_registry_state().await.expect("read state");
    let stored = state
        .entries
        .iter()
        .find(|stored| stored.run_id == entry.run_id)
        .expect("the abandoned entry is visible");
    assert_eq!(stored.phase, "retired");
    assert_eq!(
        stored.retired_ids, staged_ids,
        "the abandoned run keeps its staged ids as the tombstone"
    );
    assert!(
        stored.staged_ids.is_empty(),
        "the staged ids are cleared once they became the retired set"
    );
    assert_eq!(
        state.search_point_exclusions(),
        staged_ids,
        "a late write of a staged point stays excluded after the abandon"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_staging_entry_that_still_holds_ids_is_never_removed() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let entry = registry_entry(Uuid::new_v4(), 9);
    register_entry(&db, &entry).await;
    let epoch_before = db.read_search_registry_epoch().await.expect("epoch before");

    assert_eq!(
        delete_entry(&db, entry.run_id, "staging").await,
        None,
        "a staging row that still holds ids is not removable"
    );
    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(state.epoch, epoch_before, "a refused removal never bumps");
    assert!(
        state
            .entries
            .iter()
            .any(|stored| stored.run_id == entry.run_id),
        "the unresolved run keeps its entry"
    );

    let mut empty = registry_entry(Uuid::new_v4(), 10);
    empty.staged_ids.clear();
    empty.retired_ids.clear();
    register_entry(&db, &empty).await;
    let epoch_before = db.read_search_registry_epoch().await.expect("epoch before");
    let removed = delete_entry(&db, empty.run_id, "staging")
        .await
        .expect("an empty staging row removes");
    assert_eq!(removed, epoch_before + 1);
    let state = db.read_search_registry_state().await.expect("read state");
    assert!(
        !state
            .entries
            .iter()
            .any(|stored| stored.run_id == empty.run_id),
        "the empty staging row is gone"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_illegal_transition_is_refused_without_mutating_the_entry() {
    let db = connect_scratch().await;
    clear_registry_entries(&db).await;

    let entry = registry_entry(Uuid::new_v4(), 11);
    register_entry(&db, &entry).await;
    let epoch_before = db.read_search_registry_epoch().await.expect("epoch before");

    for (expected_phase, next_phase) in [
        ("staging", "staging"),
        ("switched", "switched"),
        ("retired", "retired"),
        ("retired", "switched"),
        ("switched", "staging"),
    ] {
        let mut tx = db.pool().begin().await.expect("begin refused transition");
        let outcome = db
            .transition_search_registry_entry_in(
                entry.run_id,
                expected_phase,
                next_phase,
                Some(&[Uuid::new_v4()]),
                &mut tx,
            )
            .await;
        assert!(
            outcome.is_err(),
            "the {expected_phase} -> {next_phase} move is not a lifecycle edge"
        );
        tx.rollback().await.expect("roll back refused transition");
    }

    let state = db.read_search_registry_state().await.expect("read state");
    assert_eq!(
        state.epoch, epoch_before,
        "a refused transition never bumps the epoch"
    );
    let stored = state
        .entries
        .iter()
        .find(|stored| stored.run_id == entry.run_id)
        .expect("the entry is retained");
    assert_eq!(
        stored, &entry,
        "a refused transition leaves the entry untouched"
    );
}
