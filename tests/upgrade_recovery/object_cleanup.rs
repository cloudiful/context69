//! Issue #975 P9: object lifecycle on the known pre-artifact recovery profile.
//!
//! The pinned recovery admission runs before the artifact migration, so
//! `context69.task_item_artifacts` does not exist and no artifact row can
//! exist. Every object GC/staging statement must therefore have a two-referrer
//! twin selected from the actual `Database::pending_upgrade()` marker, never
//! from a query error.
//!
//! This module is called from the registered upgrade fixture while it is still
//! on the pre-retirement schema. It seeds isolated objects, writes their local
//! bytes and drives the real `LibraryStore` wrappers and a real `LibraryService`
//! staged release. All fixtures are removed again so the fixture's later
//! migration and normal-profile assertions are unaffected.

use std::path::Path;

use async_trait::async_trait;
use chrono::SubsecRound;
use chrono::{Duration as ChronoDuration, Utc};
use context69::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    db::Database,
    library_store::LibraryStore,
    services::{
        library::{LibraryService, LibraryServiceConfig},
        settings::SettingsService,
    },
};
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use sqlx::PgPool;
use uuid::Uuid;

struct NoopCallbacks;

#[async_trait]
impl TranslationPublisher for NoopCallbacks {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        _translation: TranslationPublication<'_>,
    ) -> anyhow::Result<Vec<TranslationChunkPublication>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}

struct Owner {
    group_id: i64,
    user_id: i64,
}

async fn seed_owner(pool: &PgPool) -> Owner {
    let marker = Uuid::new_v4().simple().to_string();
    let group_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, 'Pre-artifact GC', 'public', 'shared', $2) RETURNING id",
    )
    .bind(format!("p9-gc-{marker}"))
    .bind(format!("test/p9-gc-{marker}"))
    .fetch_one(pool)
    .await
    .expect("seed the pre-artifact group");
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash, is_admin) \
         VALUES ($1, 'Pre-artifact GC', 'unused', FALSE) RETURNING id",
    )
    .bind(format!("p9-gc-{marker}"))
    .fetch_one(pool)
    .await
    .expect("seed the pre-artifact user");
    Owner { group_id, user_id }
}

async fn insert_object(
    pool: &PgPool,
    group_id: i64,
    content: &[u8],
    staging_lease_until: Option<chrono::DateTime<Utc>>,
) -> (Uuid, String, String) {
    let id = Uuid::new_v4();
    let sha256 = sha256_hex(content);
    let key = format!("objects/{group_id}/{sha256}");
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until) \
         VALUES ($1, $2, $3, $4, 'local', $5, $6)",
    )
    .bind(id)
    .bind(group_id)
    .bind(&sha256)
    .bind(content.len() as i64)
    .bind(&key)
    .bind(staging_lease_until)
    .execute(pool)
    .await
    .expect("seed a storage object");
    (id, key, sha256)
}

fn write_bytes(storage_root: &Path, key: &str, content: &[u8]) {
    let path = storage_root.join(key);
    std::fs::create_dir_all(path.parent().expect("object key parent"))
        .expect("create the object directory");
    std::fs::write(path, content).expect("write the object bytes");
}

fn bytes_exist(storage_root: &Path, key: &str) -> bool {
    storage_root.join(key).exists()
}

async fn object_rows(pool: &PgPool, object_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.library_storage_objects WHERE id = $1")
        .bind(object_id)
        .fetch_one(pool)
        .await
        .expect("count the object row")
}

async fn seed_file_reference(pool: &PgPool, group_id: i64, object_id: Uuid, key: &str, sha: &str) {
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, filename, media_type, size_bytes, sha256, storage_rel_path, \
          storage_object_id, ingest_status, visibility) \
         VALUES ($1, $2, $6, 'text/plain', 1, $3, $4, $5, 'succeeded', 'public')",
    )
    .bind(Uuid::new_v4())
    .bind(group_id)
    .bind(sha)
    .bind(key)
    .bind(object_id)
    .bind(format!("p9-gc-{}.txt", Uuid::new_v4().simple()))
    .execute(pool)
    .await
    .expect("seed a file reference");
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn build_service(db: &Database, storage_root: &Path) -> LibraryService {
    let settings = SettingsService::new(db.clone());
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: std::sync::Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: std::sync::Arc::new(NoopCallbacks),
    });
    LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            file_library: FileLibraryConfig {
                storage_root: storage_root.to_path_buf(),
                max_upload_size_mb: 1,
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "p9-recovery".to_string(),
        },
        settings,
        translation,
    )
    .await
    .expect("build the local-FS maintenance library")
}

async fn cleanup(pool: &PgPool, owner: &Owner) {
    for sql in [
        "DELETE FROM context69.library_files WHERE group_id = $1",
        "DELETE FROM context69.library_storage_object_cleanup WHERE group_id = $1",
        "DELETE FROM context69.library_storage_objects WHERE group_id = $1",
        "DELETE FROM context69.groups WHERE id = $1",
    ] {
        sqlx::query(sql)
            .bind(owner.group_id)
            .execute(pool)
            .await
            .expect("clean the pre-artifact fixtures");
    }
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(owner.user_id)
        .execute(pool)
        .await
        .expect("clean the pre-artifact user");
}

/// Drive every object GC/staging branch on the real pre-artifact profile.
///
/// Called only while the fixture schema is still immediately before the pinned
/// retirement migration. A missing-table error anywhere here fails the fixture.
pub(super) async fn exercise_known_profile_object_lifecycle(db: &Database, storage_root: &Path) {
    assert!(
        db.pending_upgrade().is_some(),
        "this fixture must run on the pinned pre-artifact recovery profile"
    );
    assert!(
        !db.task_item_artifact_schema_present()
            .await
            .expect("detect the artifact relation"),
        "the artifact relation must not exist on the pre-artifact profile"
    );

    let pool = db.pool();
    let owner = seed_owner(pool).await;
    let store = LibraryStore::new(db.clone());
    let storage_dir = storage_root.to_path_buf();
    std::fs::create_dir_all(&storage_dir).expect("create the local storage root");

    // 1. Upload rollback: count then guarded unreferenced delete, with real
    //    bytes the caller removes after the row is gone.
    let rollback_bytes = b"pre-artifact rollback bytes";
    let (rollback_object, rollback_key, _) =
        insert_object(pool, owner.group_id, rollback_bytes, None).await;
    write_bytes(&storage_dir, &rollback_key, rollback_bytes);
    assert_eq!(
        store
            .count_storage_object_references(rollback_object)
            .await
            .expect("two-referrer count must work before the artifact table exists"),
        0
    );
    // Fail-then-pass evidence that the profile branch is load-bearing: the
    // normal (artifact-aware) statement cannot run on this schema, while the
    // wrapper above just succeeded through its twin.
    let normal_count_error = sqlx::query(include_str!(
        "../../src/sql/library_store/objects/count_storage_object_references.sql"
    ))
    .bind(rollback_object)
    .fetch_one(pool)
    .await
    .expect_err("the artifact-aware statement must fail without the relation");
    assert!(
        normal_count_error
            .to_string()
            .contains("task_item_artifacts"),
        "the normal statement must be the one that is unusable here, got: {normal_count_error}"
    );

    let deleted = store
        .delete_unreferenced_storage_object(rollback_object)
        .await
        .expect("unreferenced delete must work in the known profile")
        .expect("an unreferenced object is deleted");
    assert_eq!(deleted.object_key, rollback_key);
    assert_eq!(deleted.storage_backend, "local");
    assert_eq!(object_rows(pool, rollback_object).await, 0);
    assert!(
        bytes_exist(&storage_dir, &rollback_key),
        "the SQL row delete never removes bytes; the caller owns the physical delete"
    );
    std::fs::remove_file(storage_dir.join(&rollback_key)).expect("remove the rolled-back bytes");

    // 2. A referenced object is refused by both the count and the delete.
    let referenced_bytes = b"pre-artifact referenced bytes";
    let (referenced_object, referenced_key, referenced_sha) =
        insert_object(pool, owner.group_id, referenced_bytes, None).await;
    write_bytes(&storage_dir, &referenced_key, referenced_bytes);
    seed_file_reference(
        pool,
        owner.group_id,
        referenced_object,
        &referenced_key,
        &referenced_sha,
    )
    .await;
    assert_eq!(
        store
            .count_storage_object_references(referenced_object)
            .await
            .expect("count the file reference"),
        1
    );
    assert!(
        store
            .delete_unreferenced_storage_object(referenced_object)
            .await
            .expect("guarded delete skips a referenced object")
            .is_none(),
        "a file-referenced object is never deleted"
    );
    assert_eq!(object_rows(pool, referenced_object).await, 1);
    assert!(bytes_exist(&storage_dir, &referenced_key));

    // 3. Staged release through the real service: stage bytes (which registers
    //    a shared staging lease), then discard through the reference-checked
    //    rollback. Both wrappers run the two-referrer twin.
    let service = build_service(db, &storage_dir).await;
    let staged = service
        .stage_maintenance_task_artifact(owner.group_id, bytes::Bytes::from_static(b"staged"))
        .await
        .expect("stage bytes in the known profile");
    assert_eq!(object_rows(pool, staged.object_id).await, 1);
    service.discard_maintenance_task_artifact(&staged).await;
    assert_eq!(
        object_rows(pool, staged.object_id).await,
        0,
        "the staged record is released without touching the artifact relation"
    );

    // 4. A still-staged object is admitted by the staged lock/delete pair, and
    //    a file-backed staged object is refused by that same pair.
    let staged_bytes = b"pre-artifact staged bytes";
    let (staged_object, staged_key, _) = insert_object(
        pool,
        owner.group_id,
        staged_bytes,
        Some(Utc::now() + ChronoDuration::hours(1)),
    )
    .await;
    write_bytes(&storage_dir, &staged_key, staged_bytes);
    let mut tx = pool.begin().await.expect("begin the staged release");
    store
        .lock_storage_object(
            &mut tx,
            &format!("{0}:{1}", owner.group_id, sha256_hex(staged_bytes)),
        )
        .await
        .expect("lock the staged object identity");
    assert!(
        store
            .get_staged_storage_object_for_update(&mut tx, staged_object)
            .await
            .expect("the staged lock must work in the known profile")
            .is_some()
    );
    assert!(
        store
            .delete_released_staged_storage_object(&mut tx, staged_object)
            .await
            .expect("the staged delete must work in the known profile")
    );
    tx.commit().await.expect("commit the staged release");
    assert_eq!(object_rows(pool, staged_object).await, 0);
    assert!(
        bytes_exist(&storage_dir, &staged_key),
        "the physical delete belongs to the service, which we bypassed here"
    );
    std::fs::remove_file(storage_dir.join(&staged_key)).expect("remove the staged bytes");

    let guarded_bytes = b"pre-artifact guarded staged bytes";
    let (guarded_object, guarded_key, guarded_sha) = insert_object(
        pool,
        owner.group_id,
        guarded_bytes,
        Some(Utc::now() + ChronoDuration::hours(1)),
    )
    .await;
    write_bytes(&storage_dir, &guarded_key, guarded_bytes);
    seed_file_reference(
        pool,
        owner.group_id,
        guarded_object,
        &guarded_key,
        &guarded_sha,
    )
    .await;
    let mut tx = pool.begin().await.expect("begin the guarded release");
    assert!(
        store
            .get_staged_storage_object_for_update(&mut tx, guarded_object)
            .await
            .expect("the guarded staged lock runs")
            .is_none(),
        "a file-referenced staged object is not releasable"
    );
    assert!(
        !store
            .delete_released_staged_storage_object(&mut tx, guarded_object)
            .await
            .expect("the guarded staged delete runs")
    );
    tx.rollback()
        .await
        .expect("release the guarded transaction");
    assert_eq!(object_rows(pool, guarded_object).await, 1);
    assert!(bytes_exist(&storage_dir, &guarded_key));

    // 5. Expired staging: a file-backed lease is cleared by the lease-clear
    //    statement, which also never deletes bytes.
    let expired_bytes = b"pre-artifact expired bytes";
    let expired_at = (Utc::now() - ChronoDuration::hours(2)).trunc_subsecs(6);
    let (expired_file_object, expired_file_key, expired_file_sha) =
        insert_object(pool, owner.group_id, expired_bytes, Some(expired_at)).await;
    write_bytes(&storage_dir, &expired_file_key, expired_bytes);
    seed_file_reference(
        pool,
        owner.group_id,
        expired_file_object,
        &expired_file_key,
        &expired_file_sha,
    )
    .await;
    assert_eq!(
        store
            .clear_expired_staging_with_file_reference(Utc::now(), 10)
            .await
            .expect("the lease-clear statement must work in the known profile"),
        1,
        "the file-backed expired lease is cleared"
    );
    let cleared_lease: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
        "SELECT staging_lease_until FROM context69.library_storage_objects WHERE id = $1",
    )
    .bind(expired_file_object)
    .fetch_one(pool)
    .await
    .expect("read the cleared lease");
    assert_eq!(cleared_lease, None);
    assert!(bytes_exist(&storage_dir, &expired_file_key));

    // 6. Expired orphan: list, under-lock re-read and under-lock record delete.
    let orphan_bytes = b"pre-artifact orphan bytes";
    let (orphan_object, orphan_key, _) =
        insert_object(pool, owner.group_id, orphan_bytes, Some(expired_at)).await;
    write_bytes(&storage_dir, &orphan_key, orphan_bytes);
    let candidates = store
        .sweep_orphaned_storage_objects(Utc::now(), 10)
        .await
        .expect("the orphan listing must work in the known profile");
    assert!(
        candidates.iter().any(|object| object.id == orphan_object),
        "the expired unreferenced object is an orphan candidate"
    );
    let mut tx = pool.begin().await.expect("begin the orphan delete");
    store
        .lock_storage_object(
            &mut tx,
            &format!("{0}:{1}", owner.group_id, sha256_hex(orphan_bytes)),
        )
        .await
        .expect("lock the orphan identity");
    assert!(
        store
            .get_storage_object_by_id_for_update(&mut tx, orphan_object, Utc::now())
            .await
            .expect("the orphan under-lock re-read must work in the known profile")
            .is_some()
    );
    assert!(
        store
            .delete_orphaned_storage_object_record_for_update(&mut tx, orphan_object, Utc::now())
            .await
            .expect("the orphan under-lock delete must work in the known profile")
    );
    tx.commit().await.expect("commit the orphan delete");
    assert_eq!(object_rows(pool, orphan_object).await, 0);
    assert!(
        bytes_exist(&storage_dir, &orphan_key),
        "physical removal belongs to the sweep, which we bypassed here"
    );
    std::fs::remove_file(storage_dir.join(&orphan_key)).expect("remove the orphan bytes");

    // 7. Source-cleanup final row delete, both on a clean object and on a
    //    file-referenced one (refused), using the per-connection wrapper.
    let cleanup_bytes = b"pre-artifact cleanup bytes";
    let (cleanup_object, cleanup_key, _) =
        insert_object(pool, owner.group_id, cleanup_bytes, None).await;
    write_bytes(&storage_dir, &cleanup_key, cleanup_bytes);
    let mut tx = pool.begin().await.expect("begin the cleanup delete");
    assert_eq!(
        store
            .count_storage_object_references_on_connection(&mut tx, cleanup_object)
            .await
            .expect("the per-connection count must work in the known profile"),
        0
    );
    assert_eq!(
        store
            .delete_cleanup_storage_object(&mut tx, cleanup_object)
            .await
            .expect("the cleanup delete must work in the known profile"),
        Some(cleanup_key.clone())
    );
    tx.commit().await.expect("commit the cleanup delete");
    assert_eq!(object_rows(pool, cleanup_object).await, 0);
    std::fs::remove_file(storage_dir.join(&cleanup_key)).expect("remove the cleanup bytes");

    let mut tx = pool
        .begin()
        .await
        .expect("begin the guarded cleanup delete");
    assert!(
        store
            .delete_cleanup_storage_object(&mut tx, guarded_object)
            .await
            .expect("the guarded cleanup delete runs")
            .is_none(),
        "a file-referenced object is never removed by the cleanup delete"
    );
    tx.rollback().await.expect("release the guarded cleanup");
    assert_eq!(object_rows(pool, guarded_object).await, 1);
    assert!(bytes_exist(&storage_dir, &guarded_key));

    // The artifact relation is still absent: no compat statement created it.
    assert!(
        !db.task_item_artifact_schema_present()
            .await
            .expect("re-detect the artifact relation"),
        "the known-profile lifecycle must never create the artifact relation"
    );

    cleanup(pool, &owner).await;
    let _ = std::fs::remove_dir_all(&storage_dir);
}
