use super::*;
use crate::{domain_errors::DomainError, pagination::PageBounds};

impl SyncService {
    pub async fn list_sources_page(
        &self,
        page: u32,
        page_size: u32,
        query: Option<&str>,
    ) -> Result<SourcePageResponse> {
        let bounds = PageBounds::new(page, page_size)
            .map_err(|error| DomainError::invalid_argument(error.to_string()))?;
        let query = query.map(str::trim).filter(|value| !value.is_empty());
        let total = self.source_store.count_sources(query).await?;
        Ok(SourcePageResponse {
            items: self
                .decorate_sources(
                    self.source_store
                        .list_sources_page(query, i64::from(bounds.page_size), bounds.offset)
                        .await?,
                )
                .await?,
            pagination: bounds.pagination(total)?,
        })
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>> {
        self.decorate_sources(self.source_store.list_sources().await?)
            .await
    }

    pub async fn list_sources_for_group(&self, group_id: i64) -> Result<Vec<SourceStatus>> {
        self.decorate_sources(self.source_store.list_sources_for_group(group_id).await?)
            .await
    }

    pub async fn get_source_for_group(
        &self,
        group_id: i64,
        source_key: &str,
    ) -> Result<Option<SourceStatus>> {
        let Some(source) = self
            .source_store
            .get_source_in_group(group_id, source_key)
            .await?
        else {
            return Ok(None);
        };
        let mut sources = self.decorate_sources(vec![source]).await?;
        Ok(sources.pop())
    }

    pub async fn create_source_in_scope(
        &self,
        scope: &crate::source_store::SourceScope,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        self.upsert_source_connection_for_source(input).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store
            .insert_source_in_scope(&source, scope)
            .await?;
        self.reload_sources().await?;
        self.get_source_for_group(scope.group_id, &source.key)
            .await?
            .ok_or_else(|| DomainError::internal(format!("missing source {}", source.key)))
            .map_err(anyhow::Error::from)
    }

    pub async fn update_source_in_group(
        &self,
        group_id: i64,
        source_key: &str,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        if input.source_key != source_key {
            return Err(DomainError::invalid_argument("source_key cannot be changed").into());
        }

        self.upsert_source_connection_for_source(input).await?;
        let _guard = self.acquire_lock(source_key).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store
            .update_source_in_group(Some(group_id), source_key, &source)
            .await?;
        self.reload_sources().await?;
        self.get_source_for_group(group_id, source_key)
            .await?
            .with_context(|| format!("missing source {source_key}"))
    }

    pub async fn delete_source_in_group(&self, group_id: i64, source_key: &str) -> Result<()> {
        let _guard = self.acquire_lock(source_key).await?;
        let chunk_ids = self.source_store.list_source_chunk_ids(source_key).await?;
        self.source_index.delete_points(&chunk_ids).await?;
        let deleted = self
            .source_store
            .delete_source_in_group(Some(group_id), source_key)
            .await?;
        if !deleted {
            return Err(DomainError::not_found(format!("unknown source {source_key}")).into());
        }
        self.reload_sources().await?;
        Ok(())
    }

    /// Removes one legacy source under the captured grant of a running item.
    ///
    /// The vector points are removed before the transaction and cannot be
    /// rolled back, so the grant is checked again right before them. The four
    /// deletes of the source then run in one guarded transaction of the grant:
    /// a worker that lost its segment removes nothing, and the manual path
    /// keeps the pooled helper above.
    pub(crate) async fn delete_source_in_group_fenced(
        &self,
        fence: crate::library_store::TaskWriteFence,
        group_id: i64,
        source_key: &str,
    ) -> Result<()> {
        let _guard = self.acquire_lock(source_key).await?;
        if !crate::library_store::task_write_granted(&self.db, fence).await? {
            return Err(anyhow::anyhow!(
                "the captured grant of the task item no longer authorizes the source delete"
            ));
        }
        let chunk_ids = self.source_store.list_source_chunk_ids(source_key).await?;
        if !chunk_ids.is_empty() {
            self.source_index.delete_points(&chunk_ids).await?;
        }
        let mut guarded = crate::library_store::begin_task_write(&self.db, fence)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "the captured grant of the task item no longer authorizes the source delete: {error}"
                )
            })?;
        let deleted = self
            .source_store
            .delete_source_in_group_in(guarded.connection(), Some(group_id), source_key)
            .await?;
        if !guarded.commit().await? {
            return Err(anyhow::anyhow!(
                "the captured grant of the task item no longer authorizes the source delete"
            ));
        }
        if !deleted {
            return Err(DomainError::not_found(format!("unknown source {source_key}")).into());
        }
        self.reload_sources().await?;
        Ok(())
    }

    async fn decorate_sources(&self, sources: Vec<SourceStatus>) -> Result<Vec<SourceStatus>> {
        let statuses = self.source_connection_statuses.read().await.clone();
        Ok(sources
            .into_iter()
            .map(|mut source| {
                if let Some(connection_status) = statuses.get(&source.connection) {
                    source.has_database_url = connection_status.has_database_url;
                    source.origin_status = connection_status.status.clone();
                    source.origin_message = connection_status.message.clone();
                }
                source
            })
            .collect())
    }

    pub async fn create_source(&self, input: &SourceConfigInput) -> Result<SourceStatus> {
        self.upsert_source_connection_for_source(input).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store.insert_source(&source).await?;
        self.reload_sources().await?;
        self.source_store
            .get_source(&source.key)
            .await?
            .with_context(|| format!("missing source {}", source.key))
    }

    pub async fn update_source(
        &self,
        source_key: &str,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        if input.source_key != source_key {
            return Err(DomainError::invalid_argument("source_key cannot be changed").into());
        }

        self.upsert_source_connection_for_source(input).await?;
        let _guard = self.acquire_lock(source_key).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store.update_source(source_key, &source).await?;
        self.reload_sources().await?;
        self.source_store
            .get_source(source_key)
            .await?
            .with_context(|| format!("missing source {source_key}"))
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<()> {
        let _guard = self.acquire_lock(source_key).await?;
        let chunk_ids = self.source_store.list_source_chunk_ids(source_key).await?;
        self.source_index.delete_points(&chunk_ids).await?;
        let deleted = self.source_store.delete_source(source_key).await?;
        if !deleted {
            return Err(DomainError::not_found(format!("unknown source {source_key}")).into());
        }
        self.reload_sources().await?;
        Ok(())
    }

    pub(crate) async fn upsert_source_connection_for_source(
        &self,
        input: &SourceConfigInput,
    ) -> Result<()> {
        let pending = self
            .resolve_source_connection(&input.connection, input.database_url.clone())
            .await?;
        self.persist_source_connection(&pending).await?;
        self.reload_sources().await?;
        Ok(())
    }
}

#[cfg(test)]
mod task_source_delete {
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::json;
    use uuid::Uuid;

    use crate::db::{CreateTaskSubmissionRequest, Database};
    use crate::library_store::TaskWriteFence;
    use crate::services::secret_store;
    use crate::services::sync::SyncService;
    use context69_translation::{
        TranslationChunkPublication, TranslationDependencies, TranslationPublication,
        TranslationPublisher, TranslationReadiness, TranslationService,
    };

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

    fn build_service(db: &Database) -> SyncService {
        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(NoopCallbacks),
            concurrency: 1,
            readiness: Arc::new(NoopCallbacks),
        });
        SyncService::new(
            db.clone(),
            secret_store::build_unkeyed(db),
            None,
            None,
            crate::chunking::ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            1,
            translation,
        )
    }

    /// A group, a claimed task item and its captured grant.
    async fn seed_claim(db: &Database) -> (i64, Uuid, Uuid, i64) {
        let user_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_user.sql",
            format!("p3d-source-{}", Uuid::new_v4()),
            "P3D Source",
            "unused"
        )
        .fetch_one(db.pool())
        .await
        .expect("seed the user");
        let group_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_group.sql",
            format!("p3d-source-{}", Uuid::new_v4()),
            "P3D Source",
            format!("test/p3d-source-{}", Uuid::new_v4())
        )
        .fetch_one(db.pool())
        .await
        .expect("seed the group");
        sqlx::query_file!(
            "tests/sql/task_execution/seed_group_membership.sql",
            group_id,
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
                group_id: Some(group_id),
                kind: "text_batch",
                group_path: Some("test/p3d-source"),
                source_key: None,
                payloads: &[json!({"external_id": "a"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("p3d-source-{}", Uuid::new_v4()),
            })
            .await
            .expect("create the task");
        (group_id, task_id, items[0], user_id)
    }

    /// Crashes the worker that holds the item: the authoritative library attempt
    /// and its domain mirror expire together.
    async fn crash(db: &Database, fence: TaskWriteFence) {
        sqlx::query_file!(
            "tests/sql/task_execution/expire_library_attempt_by_fence.sql",
            fence.execution_task_id(),
            fence.token()
        )
        .execute(db.pool())
        .await
        .expect("expire the library attempt");
        sqlx::query_file!(
            "tests/sql/task_execution/expire_item_lease.sql",
            fence.item_id()
        )
        .execute(db.pool())
        .await
        .expect("expire the domain mirror");
    }

    async fn count_source_configs(db: &Database, group_id: i64) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_group_source_configs.sql",
            group_id
        )
        .fetch_one(db.pool())
        .await
        .expect("count the source configs")
    }

    async fn count_source_checkpoints(db: &Database, group_id: i64) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_group_source_checkpoints.sql",
            group_id
        )
        .fetch_one(db.pool())
        .await
        .expect("count the source checkpoints")
    }

    async fn count_sync_runs(db: &Database, group_id: i64) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_group_sync_runs.sql",
            group_id
        )
        .fetch_one(db.pool())
        .await
        .expect("count the sync runs")
    }

    async fn cleanup(db: &Database, group_id: i64, task_id: Uuid, user_id: i64) {
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_sync_runs.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete the sync runs");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_source_checkpoints.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete the source checkpoints");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_source_configs.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete the source configs");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_task_items_by_task.sql",
            task_id
        )
        .execute(db.pool())
        .await
        .expect("delete the task items");
        sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
            .execute(db.pool())
            .await
            .expect("delete the task");
        sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group_id)
            .execute(db.pool())
            .await
            .expect("delete the group");
        sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
            .execute(db.pool())
            .await
            .expect("delete the user");
    }

    /// The sync state and the legacy source of one source folder are removed
    /// under the captured grant, and a stale grant removes nothing.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_stale_grant_removes_no_source_state_and_a_live_grant_removes_all_of_it() {
        let db = connect().await;
        let (group_id, task_id, item_id, user_id) = seed_claim(&db).await;
        let service = build_service(&db);
        let source_key = format!("p3d-source-{}", Uuid::new_v4());
        let identity = format!("p3d-identity-{}", Uuid::new_v4());

        sqlx::query_file!(
            "tests/sql/task_execution/seed_source_config.sql",
            group_id,
            &source_key
        )
        .execute(db.pool())
        .await
        .expect("seed the legacy source");
        sqlx::query_file!(
            "tests/sql/task_execution/seed_source_checkpoint.sql",
            &identity,
            group_id
        )
        .execute(db.pool())
        .await
        .expect("seed the checkpoint");
        sqlx::query_file!(
            "tests/sql/task_execution/seed_sync_run.sql",
            &identity,
            group_id
        )
        .execute(db.pool())
        .await
        .expect("seed the run");

        // The service registry learns the seeded source the way a real sync
        // does, so a lock can be taken for it.
        service
            .reload_sources()
            .await
            .expect("reload the source registry");

        // The claim is lost before the delete: nothing may be removed.
        let held = claim_item(&db, item_id).await;
        let stale = TaskWriteFence::from_claimed_item(&held).expect("the captured grant");
        crash(&db, stale).await;
        let refused = service
            .delete_project_source_folder_state_fenced(
                stale,
                group_id,
                &identity,
                Some(&source_key),
            )
            .await;
        assert!(
            refused.is_err(),
            "a stale grant cannot remove the source state"
        );
        assert_eq!(
            count_source_configs(&db, group_id).await,
            1,
            "the legacy source survives the stale grant"
        );
        assert_eq!(
            count_source_checkpoints(&db, group_id).await,
            1,
            "the checkpoint survives the stale grant"
        );
        assert_eq!(
            count_sync_runs(&db, group_id).await,
            1,
            "the sync run survives the stale grant"
        );

        // A live owner removes all of it under its grant.
        let recovered = claim_item(&db, item_id).await;
        let live = TaskWriteFence::from_claimed_item(&recovered).expect("the captured grant");
        assert!(
            live.generation() > stale.generation(),
            "a newer generation took over"
        );
        service
            .delete_project_source_folder_state_fenced(live, group_id, &identity, Some(&source_key))
            .await
            .expect("the live grant removes the source state");
        assert_eq!(
            count_source_configs(&db, group_id).await,
            0,
            "the legacy source is removed"
        );
        assert_eq!(
            count_source_checkpoints(&db, group_id).await,
            0,
            "the checkpoint is removed"
        );
        assert_eq!(
            count_sync_runs(&db, group_id).await,
            0,
            "the sync run is removed"
        );

        cleanup(&db, group_id, task_id, user_id).await;
    }

    async fn claim_item(db: &Database, item_id: Uuid) -> crate::db::ClaimedItem {
        db.claim_items_fast(50)
            .await
            .expect("claim the item")
            .into_iter()
            .find(|item| item.id == item_id)
            .expect("the item is claimed")
    }
}
