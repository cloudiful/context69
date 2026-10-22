use anyhow::{Context, Result};
use tracing::warn;
use uuid::Uuid;

use super::LibraryService;
use super::*;

/// Grouped arguments for rolling back a project file change.
#[derive(Debug, Clone, Copy)]
pub struct RollbackProjectFileChangeRequest<'a> {
    pub project_id: i64,
    pub file_id: Uuid,
    pub previous_file: Option<&'a crate::domain::LibraryFileRecord>,
    pub previous_storage: Option<&'a crate::library_store::documents::StoragePathRow>,
    pub previous_translation: Option<&'a crate::contracts::TranslationDirective>,
    pub new_storage_key: &'a str,
    pub new_storage_object_id: Option<Uuid>,
    pub lease_token: Option<Uuid>,
}

impl LibraryService {
    pub(super) async fn rollback_new_file_record_for_task(
        &self,
        project_id: Option<i64>,
        file_id: Uuid,
        storage_key: Option<&str>,
        storage_object_id: Option<Uuid>,
        lease_token: Uuid,
    ) {
        self.rollback_new_file_record_with_lease(
            project_id,
            file_id,
            storage_key,
            storage_object_id,
            Some(lease_token),
        )
        .await;
    }

    pub(super) async fn rollback_new_uploaded_file_for_task(
        &self,
        project_id: i64,
        file_id: Uuid,
        storage_object: &crate::library_store::objects::StorageObjectRecord,
        lease_token: Uuid,
    ) {
        self.rollback_new_file_record_for_task(
            Some(project_id),
            file_id,
            Some(&storage_object.object_key),
            Some(storage_object.id),
            lease_token,
        )
        .await;
    }

    async fn rollback_new_file_record_with_lease(
        &self,
        project_id: Option<i64>,
        file_id: Uuid,
        storage_key: Option<&str>,
        storage_object_id: Option<Uuid>,
        lease_token: Option<Uuid>,
    ) {
        let removed = match project_id {
            Some(project_id) => {
                self.store
                    .delete_file_record_in_project(project_id, file_id)
                    .await
            }
            None => self.store.delete_file_record(file_id).await,
        };
        match removed {
            Ok(true) => {}
            Ok(false) => warn!(file_id = %file_id, "file rollback found no database record"),
            Err(error) => {
                warn!(file_id = %file_id, %error, "failed to remove file record after upload failure")
            }
        }

        if let Some(storage_object_id) = storage_object_id {
            match lease_token {
                Some(lease_token) => {
                    self.delete_unreferenced_storage_object_for_lease(
                        storage_object_id,
                        lease_token,
                    )
                    .await
                }
                None => {
                    self.delete_unreferenced_storage_object(storage_object_id)
                        .await
                }
            }
        } else if let Some(storage_key) = storage_key {
            let result = match lease_token {
                Some(lease_token) => {
                    self.delete_active_storage_for_lease(storage_key, lease_token)
                        .await
                }
                None => self.delete_active_storage(storage_key).await,
            };
            if let Err(error) = result {
                warn!(file_id = %file_id, %error, "failed to remove storage object after file rollback");
            }
        }
    }

    pub(super) async fn delete_unreferenced_storage_object(&self, object_id: Uuid) {
        self.delete_unreferenced_storage_object_with_lease(object_id, None)
            .await;
    }

    pub(super) async fn delete_unreferenced_storage_object_for_lease(
        &self,
        object_id: Uuid,
        lease_token: Uuid,
    ) {
        self.delete_unreferenced_storage_object_with_lease(object_id, Some(lease_token))
            .await;
    }

    async fn delete_unreferenced_storage_object_with_lease(
        &self,
        object_id: Uuid,
        lease_token: Option<Uuid>,
    ) {
        match self
            .store
            .delete_unreferenced_storage_object(object_id)
            .await
        {
            Ok(Some(object)) if object.storage_backend == self.storage.backend() => {
                let result = match lease_token {
                    Some(lease_token) => {
                        self.delete_active_storage_for_lease(&object.object_key, lease_token)
                            .await
                    }
                    None => self.delete_active_storage(&object.object_key).await,
                };
                if let Err(error) = result {
                    warn!(
                        object_id = %object_id,
                        object_key = %object.object_key,
                        %error,
                        "failed to remove unreferenced library storage object"
                    );
                }
            }
            Ok(Some(object)) => {
                warn!(
                    object_id = %object_id,
                    storage_backend = %object.storage_backend,
                    active_storage_backend = self.storage.backend(),
                    "unreferenced library storage object belongs to an inactive backend"
                );
            }
            Ok(None) => {}
            Err(error) => {
                warn!(object_id = %object_id, %error, "failed to remove unreferenced library storage object record");
            }
        }
    }

    pub(super) async fn restore_project_file_snapshot(
        &self,
        file: &crate::domain::LibraryFileRecord,
        storage_object_id: Option<Uuid>,
        source_released_at: Option<chrono::DateTime<chrono::Utc>>,
        translation: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<()> {
        self.store
            .restore_file_snapshot_in_project(file, storage_object_id, source_released_at)
            .await?
            .with_context(|| format!("unknown file {} while restoring upload", file.id))?;
        let restore_metadata_json = file
            .metadata_json
            .as_object()
            .map(|map| {
                map.iter()
                    .map(|(key, val)| (key.clone(), val.clone()))
                    .collect()
            })
            .unwrap_or_default();
        self.store
            .update_business_metadata(
                file.group_id,
                file.id,
                &crate::contracts::CanonicalUploadMetadata {
                    external_id: file.external_id.clone(),
                    source_uri: file.source_uri.clone(),
                    published_at: file.published_at,
                    metadata_json: restore_metadata_json,
                },
            )
            .await?
            .with_context(|| format!("unknown file {} while restoring metadata", file.id))?;
        self.store
            .set_file_translation_directive(file.id, translation)
            .await?;
        self.refresh_metadata_for_file(file.id).await?;
        self.bump_search_generation("library upload rollback")
            .await?;
        Ok(())
    }

    /// The compensation of one failed task upload, under the original
    /// captured grant.
    ///
    /// The previous snapshot and the search generation counter are restored in
    /// one guarded transaction of the grant, and the replacement storage object
    /// is only released once that transaction committed: a worker that lost its
    /// segment compensates nothing, so it can neither overwrite the row of a
    /// newer owner nor invalidate the search cache for it. Returns `true` when
    /// the grant still held and the compensation committed.
    ///
    /// The pooled [`Self::rollback_project_file_change`] is never reached from
    /// a task: a refused guarded compensation is reported, not retried.
    pub(super) async fn rollback_project_file_change_for_task(
        &self,
        fence: crate::library_store::TaskWriteFence,
        request: RollbackProjectFileChangeRequest<'_>,
    ) -> bool {
        let RollbackProjectFileChangeRequest {
            project_id,
            file_id,
            previous_file,
            previous_storage,
            previous_translation,
            new_storage_key,
            new_storage_object_id,
            lease_token: _,
        } = request;
        if let Some(previous_file) = previous_file {
            let restore_metadata_json = previous_file
                .metadata_json
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(key, val)| (key.clone(), val.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let metadata = crate::contracts::CanonicalUploadMetadata {
                external_id: previous_file.external_id.clone(),
                source_uri: previous_file.source_uri.clone(),
                published_at: previous_file.published_at,
                metadata_json: restore_metadata_json,
            };
            let restored = match self
                .store
                .restore_file_snapshot_for_task_fenced(
                    fence,
                    previous_file,
                    previous_storage.and_then(|path| path.storage_object_id),
                    previous_storage.and_then(|path| path.source_released_at),
                    &metadata,
                    previous_translation,
                )
                .await
            {
                Ok(restored) => restored,
                Err(error) => {
                    warn!(
                        file_id = %file_id,
                        %error,
                        "failed to restore the file snapshot of a failed task upload"
                    );
                    return false;
                }
            };
            if !restored {
                return false;
            }
            self.release_replacement_storage_for_task(
                fence,
                file_id,
                new_storage_key,
                new_storage_object_id,
            )
            .await;
            return true;
        }

        match self
            .store
            .rollback_file_record_for_task_fenced(fence, project_id, file_id)
            .await
        {
            Ok(true) => {
                self.release_replacement_storage_for_task(
                    fence,
                    file_id,
                    new_storage_key,
                    new_storage_object_id,
                )
                .await;
                true
            }
            Ok(false) => false,
            Err(error) => {
                warn!(
                    file_id = %file_id,
                    %error,
                    "failed to remove the file record of a failed task upload"
                );
                false
            }
        }
    }

    /// Releases the storage object of the replacement the failed task wrote.
    ///
    /// The bytes are only released after the guarded compensation committed, and
    /// the release itself is reference-safe: an object a successor already
    /// references is not removed.
    async fn release_replacement_storage_for_task(
        &self,
        fence: crate::library_store::TaskWriteFence,
        file_id: Uuid,
        new_storage_key: &str,
        new_storage_object_id: Option<Uuid>,
    ) {
        if let Some(object_id) = new_storage_object_id {
            self.delete_unreferenced_storage_object_for_lease(object_id, fence.token())
                .await;
        } else {
            if let Err(error) = self
                .delete_active_storage_for_lease(new_storage_key, fence.token())
                .await
            {
                warn!(
                    file_id = %file_id,
                    path = %new_storage_key,
                    %error,
                    "failed to remove the replacement storage object of a failed task upload"
                );
            }
        }
    }

    pub(super) async fn rollback_project_file_change(
        &self,
        request: RollbackProjectFileChangeRequest<'_>,
    ) {
        let RollbackProjectFileChangeRequest {
            project_id,
            file_id,
            previous_file,
            previous_storage,
            previous_translation,
            new_storage_key,
            new_storage_object_id,
            lease_token,
        } = request;
        if let Some(previous_file) = previous_file {
            if let Err(error) = self
                .restore_project_file_snapshot(
                    previous_file,
                    previous_storage.and_then(|path| path.storage_object_id),
                    previous_storage.and_then(|path| path.source_released_at),
                    previous_translation,
                )
                .await
            {
                warn!(
                    file_id = %file_id,
                    %error,
                    "failed to restore project file after upload failure"
                );
            }
            if let Some(object_id) = new_storage_object_id {
                match lease_token {
                    Some(lease_token) => {
                        self.delete_unreferenced_storage_object_for_lease(object_id, lease_token)
                            .await
                    }
                    None => self.delete_unreferenced_storage_object(object_id).await,
                }
            } else {
                let result = match lease_token {
                    Some(lease_token) => {
                        self.delete_active_storage_for_lease(new_storage_key, lease_token)
                            .await
                    }
                    None => self.delete_active_storage(new_storage_key).await,
                };
                if let Err(error) = result {
                    warn!(
                        file_id = %file_id,
                        path = %new_storage_key,
                        %error,
                        "failed to remove replacement storage object"
                    );
                }
            }
            return;
        }

        self.rollback_new_file_record_with_lease(
            Some(project_id),
            file_id,
            Some(new_storage_key),
            new_storage_object_id,
            lease_token,
        )
        .await;
    }

    pub(super) async fn finalize_uploaded_file_for_task(
        &self,
        rollback: UploadedLibraryFileRollback,
        lease_token: Uuid,
    ) {
        if let Err(error) = self
            .delete_unreferenced_objects_with_lease(rollback.old_storage_paths, Some(lease_token))
            .await
        {
            warn!(%error, "failed to remove replaced library storage object");
        }
    }
}

#[cfg(test)]
mod task_compensation {
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::json;
    use uuid::Uuid;

    use crate::chunking::ChunkingConfig;
    use crate::config::FileLibraryConfig;
    use crate::db::{CreateTaskSubmissionRequest, Database};
    use crate::library_store::TaskWriteFence;
    use crate::services::library::{LibraryService, LibraryServiceConfig};
    use crate::services::settings::SettingsService;
    use context69_namespace::GroupRecord;
    use context69_translation::{
        TranslationChunkPublication, TranslationDependencies, TranslationPublication,
        TranslationPublisher, TranslationReadiness, TranslationService,
    };

    use super::RollbackProjectFileChangeRequest;

    struct NoopCallbacks;

    #[async_trait]
    impl TranslationPublisher for NoopCallbacks {
        async fn publish(
            &self,
            _old_chunk_ids: &[Uuid],
            _translation: TranslationPublication<'_>,
        ) -> Result<Vec<TranslationChunkPublication>> {
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

    async fn connect() -> Database {
        let url = context69_test_support::accessor::database_url();
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap the library schema");
        db
    }

    async fn build_service(db: &Database) -> LibraryService {
        let storage_root = std::env::temp_dir().join(format!("context69-p3e-{}", Uuid::new_v4()));
        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(NoopCallbacks),
            concurrency: 1,
            readiness: Arc::new(NoopCallbacks),
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
                    storage_root,
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
                embedding_vector_configuration_fingerprint: "p3e-compensation".to_string(),
            },
            SettingsService::new(db.clone()),
            translation,
        )
        .await
        .expect("build the library service")
    }

    async fn seed_group_record(db: &Database) -> GroupRecord {
        let key = format!("p3e-compensation-{}", Uuid::new_v4());
        let row = sqlx::query_file!(
            "tests/sql/task_execution/seed_group_returning.sql",
            &key,
            "P3E Compensation",
            format!("test/{key}")
        )
        .fetch_one(db.pool())
        .await
        .expect("seed the group");
        GroupRecord {
            id: row.id,
            parent_group_id: None,
            group_key: row.group_key,
            group_path: row.full_path,
            parent_group_path: None,
            name: row.name,
            visibility: context69_contracts::Visibility::Public,
            kind: context69_contracts::GroupKind::Shared,
            owner_user_id: None,
            created_at: row.created_at,
            updated_at: row.updated_at,
            current_role: None,
        }
    }

    /// A claimed task item and its captured grant.
    async fn seed_claim(db: &Database, group: &GroupRecord) -> (Uuid, Uuid) {
        let user_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_user.sql",
            format!("p3e-{}", Uuid::new_v4()),
            "P3E User",
            "unused"
        )
        .fetch_one(db.pool())
        .await
        .expect("seed the user");
        sqlx::query_file!(
            "tests/sql/task_execution/seed_group_membership.sql",
            group.id,
            user_id
        )
        .execute(db.pool())
        .await
        .expect("seed the membership");
        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: Some(group.id),
                kind: "text_batch",
                group_path: Some("test/p3e"),
                source_key: None,
                payloads: &[json!({"external_id": "a"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("p3e-{}", Uuid::new_v4()),
            })
            .await
            .expect("create the task");
        (task_id, items[0])
    }

    async fn claim(db: &Database, item_id: Uuid) -> crate::db::ClaimedItem {
        db.claim_items_fast(50)
            .await
            .expect("claim the item")
            .into_iter()
            .find(|item| item.id == item_id)
            .expect("the item is claimed")
    }

    /// Crashes the worker: the authoritative attempt and its mirror expire.
    async fn crash(db: &Database, item: &crate::db::ClaimedItem) {
        let execution = item
            .execution_context()
            .expect("the claimed item carries its captured execution");
        sqlx::query_file!(
            "tests/sql/task_execution/expire_library_attempt_by_id.sql",
            execution.attempt_id()
        )
        .execute(db.pool())
        .await
        .expect("expire the library attempt");
        sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item.id)
            .execute(db.pool())
            .await
            .expect("expire the domain mirror");
    }

    async fn search_generation(db: &Database) -> i64 {
        sqlx::query_file_scalar!("tests/sql/task_execution/read_global_search_generation.sql",)
            .fetch_one(db.pool())
            .await
            .expect("read the search generation")
    }

    async fn file_row(db: &Database, file_id: Uuid) -> (String, String, i64, Option<String>) {
        let row = sqlx::query_file!(
            "tests/sql/task_execution/read_file_snapshot_columns.sql",
            file_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read the file row");
        (
            row.sha256,
            row.storage_rel_path,
            row.size_bytes,
            row.external_id,
        )
    }

    async fn cleanup(db: &Database, group_id: i64, task_id: Uuid, item_ids: &[Uuid]) {
        for id in item_ids {
            sqlx::query_file!("tests/sql/task_execution/delete_task_item_by_id.sql", id)
                .execute(db.pool())
                .await
                .expect("delete the item");
        }
        sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
            .execute(db.pool())
            .await
            .expect("delete the task");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_file_document_links.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete the file document links");
        sqlx::query_file!("tests/sql/task_execution/delete_group_files.sql", group_id)
            .execute(db.pool())
            .await
            .expect("delete the files");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_storage_objects.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete the storage objects");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_memberships.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete the memberships");
        sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group_id)
            .execute(db.pool())
            .await
            .expect("delete the group");
        sqlx::query_file!("tests/sql/task_execution/delete_p3e_users_by_login.sql")
            .execute(db.pool())
            .await
            .expect("delete the users");
    }

    fn named_text_request(content: &str) -> crate::services::library::UpsertNamedTextFileRequest {
        crate::services::library::UpsertNamedTextFileRequest {
            folder_id: None,
            external_id: "p3e-compensation".to_string(),
            filename: "p3e.txt".to_string(),
            media_type: "text/plain".to_string(),
            content: content.to_string(),
        }
    }

    /// A valid grant restores the previous snapshot and the counter in one
    /// guarded write, and the restored row is the captured one.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_valid_grant_restores_the_previous_snapshot_and_the_counter() {
        let db = connect().await;
        let service = build_service(&db).await;
        let group = seed_group_record(&db).await;
        let (task_id, item_id) = seed_claim(&db, &group).await;

        // The previous content of the file.
        let previous = service
            .upsert_named_text_file_in_project(&group, &named_text_request("old content"))
            .await
            .expect("write the previous content");
        let previous_file = service
            .store
            .get_file(previous.file_id)
            .await
            .expect("read the file")
            .expect("the file exists");

        // A successor writes different content over the same external id.
        service
            .upsert_named_text_file_in_project(&group, &named_text_request("successor content"))
            .await
            .expect("write the successor content");
        let successor = file_row(&db, previous.file_id).await;
        assert_ne!(
            successor.0, previous_file.sha256,
            "the successor changed the row"
        );

        let item = claim(&db, item_id).await;
        let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
        let generation_before = search_generation(&db).await;

        let restored = service
            .rollback_project_file_change_for_task(
                fence,
                RollbackProjectFileChangeRequest {
                    project_id: group.id,
                    file_id: previous.file_id,
                    previous_file: Some(&previous_file),
                    previous_storage: None,
                    previous_translation: None,
                    new_storage_key: &successor.1,
                    new_storage_object_id: None,
                    lease_token: None,
                },
            )
            .await;
        assert!(restored, "a valid grant compensates the failed write");

        let after = file_row(&db, previous.file_id).await;
        assert_eq!(
            after.0, previous_file.sha256,
            "the previous content hash is restored"
        );
        assert_eq!(
            after.3, previous_file.external_id,
            "the previous external id is restored"
        );
        assert_eq!(
            search_generation(&db).await,
            generation_before + 1,
            "the compensation advances the counter exactly once"
        );

        cleanup(&db, group.id, task_id, &[item_id]).await;
    }

    /// A superseded grant compensates nothing: the successor's row and the
    /// counter stay exactly as they were.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_superseded_grant_restores_nothing_over_the_successor() {
        let db = connect().await;
        let service = build_service(&db).await;
        let group = seed_group_record(&db).await;
        let (task_id, item_id) = seed_claim(&db, &group).await;

        let previous = service
            .upsert_named_text_file_in_project(&group, &named_text_request("old content"))
            .await
            .expect("write the previous content");
        let previous_file = service
            .store
            .get_file(previous.file_id)
            .await
            .expect("read the file")
            .expect("the file exists");
        service
            .upsert_named_text_file_in_project(&group, &named_text_request("successor content"))
            .await
            .expect("write the successor content");
        let successor = file_row(&db, previous.file_id).await;
        let generation_before = search_generation(&db).await;

        let item = claim(&db, item_id).await;
        let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
        crash(&db, &item).await;

        let restored = service
            .rollback_project_file_change_for_task(
                fence,
                RollbackProjectFileChangeRequest {
                    project_id: group.id,
                    file_id: previous.file_id,
                    previous_file: Some(&previous_file),
                    previous_storage: None,
                    previous_translation: None,
                    new_storage_key: &successor.1,
                    new_storage_object_id: None,
                    lease_token: None,
                },
            )
            .await;
        assert!(
            !restored,
            "a stale grant compensates nothing and reports the ownership loss"
        );
        assert_eq!(
            file_row(&db, previous.file_id).await,
            successor,
            "the successor's row is preserved"
        );
        assert_eq!(
            search_generation(&db).await,
            generation_before,
            "the stale compensation does not advance the counter"
        );

        cleanup(&db, group.id, task_id, &[item_id]).await;
    }

    /// The real task upsert path drives the same compensation and refuses it
    /// when the grant is already gone, without touching the row.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_task_upsert_path_compensates_nothing_when_the_grant_is_gone() {
        let db = connect().await;
        let service = build_service(&db).await;
        let group = seed_group_record(&db).await;
        let (task_id, item_id) = seed_claim(&db, &group).await;

        let previous = service
            .upsert_named_text_file_in_project(&group, &named_text_request("old content"))
            .await
            .expect("write the previous content");
        let successor = file_row(&db, previous.file_id).await;
        let generation_before = search_generation(&db).await;

        let item = claim(&db, item_id).await;
        let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
        crash(&db, &item).await;

        let refused = service
            .upsert_named_text_file_for_task(&group, &named_text_request("new content"), fence)
            .await;
        assert!(refused.is_err(), "the task upsert is refused");
        assert_eq!(
            file_row(&db, previous.file_id).await,
            successor,
            "the compensation of the refused upsert changed no row"
        );
        assert_eq!(
            search_generation(&db).await,
            generation_before,
            "the refused upsert advanced no counter"
        );

        cleanup(&db, group.id, task_id, &[item_id]).await;
    }
}
