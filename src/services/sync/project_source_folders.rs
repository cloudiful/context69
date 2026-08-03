use std::sync::Arc;

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::{
    config::{SourceConfig, SyncStrategy},
    contracts::SyncOutcome,
    domain::SyncCheckpoint,
    normalize::normalize_record,
    services::{
        library::{LibraryService, UpsertNamedTextFileRequest},
        source_folders::{
            serialize_source_record, source_folder_identity, source_record_external_id,
            source_record_filename,
        },
    },
    sources::{SourceConnector, postgres_sql::PostgresSqlSourceConnector},
};

use super::*;

struct ProjectSourceFolderSync<'a> {
    project: &'a crate::domain::GroupRecord,
    identity: &'a str,
    folder_id: Uuid,
    records_folder_id: Uuid,
    source: &'a SourceConfig,
    connector: Arc<dyn SourceConnector>,
    library: &'a LibraryService,
    lease_token: Uuid,
}

impl SyncService {
    pub(crate) async fn sync_project_source_folder(
        &self,
        project: &crate::domain::GroupRecord,
        folder_path: &str,
        folder_id: Uuid,
        records_folder_id: Uuid,
        source: &SourceConfig,
        library: &LibraryService,
        lease_token: Uuid,
    ) -> Result<SyncOutcome> {
        let identity = source_folder_identity(project.id, folder_path);

        if source.sync_strategy == SyncStrategy::Cursor
            && self
                .db
                .get_checkpoint(&identity)
                .await?
                .updated_at
                .is_none()
        {
            self.try_migrate_legacy_checkpoint(project.id, &identity, &source.key)
                .await?;
        }

        let connector = self.project_source_connector(source).await?;
        let run = self
            .db
            .start_run_in_scope(project.id, project.visibility, &identity, "api")
            .await?;

        match self
            .sync_project_source_folder_inner(ProjectSourceFolderSync {
                project,
                identity: &identity,
                folder_id,
                records_folder_id,
                source,
                connector,
                library,
                lease_token,
            })
            .await
        {
            Ok(outcome) => {
                self.db
                    .finish_run(&run, "completed", &outcome, None)
                    .await?;
                Ok(outcome)
            }
            Err(error) => {
                let empty_outcome = SyncOutcome {
                    records_seen: 0,
                    records_changed: 0,
                    chunks_upserted: 0,
                };
                self.db
                    .finish_run(&run, "failed", &empty_outcome, Some(&error.to_string()))
                    .await?;
                Err(error)
            }
        }
    }

    pub(crate) async fn delete_project_source_folder_state(
        &self,
        project_id: i64,
        identity: &str,
        legacy_source_key: Option<&str>,
    ) -> Result<()> {
        self.db
            .delete_sync_state_in_project(project_id, identity)
            .await?;
        if let Some(legacy_source_key) = legacy_source_key
            && self
                .source_store
                .get_source_in_group(project_id, legacy_source_key)
                .await?
                .is_some()
        {
            self.delete_source_in_group(project_id, legacy_source_key)
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn rename_project_source_folder_identity(
        &self,
        project_id: i64,
        old_identity: &str,
        new_identity: &str,
    ) -> Result<()> {
        self.db
            .rename_sync_state_in_project(project_id, old_identity, new_identity)
            .await
    }

    async fn sync_project_source_folder_inner(
        &self,
        context: ProjectSourceFolderSync<'_>,
    ) -> Result<SyncOutcome> {
        let ProjectSourceFolderSync {
            project,
            identity,
            folder_id,
            records_folder_id,
            source,
            connector,
            library,
            lease_token,
        } = context;
        let persisted_checkpoint = if source.sync_strategy == SyncStrategy::Cursor {
            self.db.get_checkpoint(identity).await?
        } else {
            SyncCheckpoint {
                updated_at: None,
                external_id: None,
            }
        };
        let mut local_checkpoint = persisted_checkpoint.clone();
        let mut outcome = SyncOutcome {
            records_seen: 0,
            records_changed: 0,
            chunks_upserted: 0,
        };
        let mut seen_external_ids = Vec::new();

        loop {
            let batch = connector.fetch_batch(&local_checkpoint).await?;
            if batch.is_empty() {
                break;
            }

            for record in batch {
                outcome.records_seen += 1;
                let normalized = normalize_record(record);
                seen_external_ids.push(normalized.external_id.clone());

                let content = serialize_source_record(&crate::domain::SourceRecord {
                    external_id: normalized.external_id.clone(),
                    title: normalized.title.clone(),
                    body_text: normalized.body_text.clone(),
                    source_uri: normalized.source_uri.clone(),
                    summary: normalized.summary.clone(),
                    published_at: normalized.published_at,
                    updated_at: normalized.updated_at,
                    metadata_json: normalized.metadata_json.clone(),
                })?;
                let (response, section_payload) = library
                    .upsert_named_text_file_for_task(
                        project,
                        &UpsertNamedTextFileRequest {
                            folder_id: Some(records_folder_id),
                            external_id: source_record_external_id(
                                folder_id,
                                &normalized.external_id,
                            ),
                            filename: source_record_filename(&normalized.external_id),
                            media_type: "application/json".to_string(),
                            content,
                        },
                        lease_token,
                    )
                    .await?;
                library
                    .mark_file_running_for_task(response.file_id)
                    .await
                    .map_err(anyhow::Error::new)?;
                if let Err(failure) = library
                    .persist_file_sections_for_task(response.file_id, &section_payload, lease_token)
                    .await
                {
                    let failure = library
                        .handle_task_ingest_failure(response.file_id, lease_token, failure)
                        .await;
                    return Err(anyhow::Error::new(failure));
                }
                outcome.records_changed += 1;
                outcome.chunks_upserted += 1;

                local_checkpoint.updated_at = Some(normalized.updated_at);
                local_checkpoint.external_id = Some(normalized.external_id);
            }
        }

        if source.sync_strategy == SyncStrategy::FullScan {
            let stale_files = library
                .list_file_records_in_project(project)
                .await?
                .into_iter()
                .filter(|file| file.folder_id == Some(records_folder_id))
                .filter(|file| {
                    file.external_id.as_deref().is_some_and(|value| {
                        value.starts_with(&format!("source-folder:record:{folder_id}:"))
                    })
                })
                .filter(|file| {
                    let Some(external_id) = file.external_id.as_deref() else {
                        return false;
                    };
                    let raw_external_id = external_id
                        .splitn(4, ':')
                        .nth(3)
                        .unwrap_or_default()
                        .to_string();
                    !seen_external_ids.contains(&raw_external_id)
                })
                .collect::<Vec<_>>();
            for file in stale_files {
                library
                    .delete_file_in_project_for_task(project, file.id, lease_token)
                    .await?;
                outcome.records_changed += 1;
            }
        }

        if source.sync_strategy == SyncStrategy::Cursor {
            self.db
                .save_checkpoint_in_scope(
                    project.id,
                    project.visibility,
                    identity,
                    &local_checkpoint,
                )
                .await?;
        } else {
            self.db
                .delete_sync_state_in_project(project.id, identity)
                .await?;
        }

        if outcome.records_changed > 0 {
            let legacy = self
                .source_store
                .get_source_in_group(project.id, &source.key)
                .await?;
            if legacy.is_some() {
                self.delete_source_in_group(project.id, &source.key).await?;
            }
        }

        Ok(outcome)
    }

    async fn try_migrate_legacy_checkpoint(
        &self,
        project_id: i64,
        identity: &str,
        legacy_source_key: &str,
    ) -> Result<()> {
        if self
            .source_store
            .get_source_in_group(project_id, legacy_source_key)
            .await?
            .is_none()
        {
            return Ok(());
        }
        let checkpoint = self.db.get_checkpoint(legacy_source_key).await?;
        if checkpoint.updated_at.is_none() && checkpoint.external_id.is_none() {
            return Ok(());
        }
        let scope = self
            .source_store
            .get_source_scope(legacy_source_key)
            .await?
            .with_context(|| format!("missing source scope for {legacy_source_key}"))?;
        self.db
            .save_checkpoint_in_scope(scope.group_id, scope.visibility, identity, &checkpoint)
            .await
    }

    async fn project_source_connector(
        &self,
        source: &SourceConfig,
    ) -> Result<Arc<dyn SourceConnector>> {
        let pool = self
            .source_pools
            .read()
            .await
            .get(&source.connection)
            .cloned()
            .with_context(|| format!("source origin is unavailable for {}", source.key))?;
        let connector = PostgresSqlSourceConnector::new(
            pool,
            source.key.clone(),
            source.sync_strategy,
            source.connector.clone(),
        );
        Ok(Arc::new(connector))
    }
}
