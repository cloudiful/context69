use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};

use anyhow::{Context, Result};
use serde_json::json;
use uuid::Uuid;

use crate::{
    contracts::{
        CreateFolderRequest, CreateSourceFolderRequest, MoveFolderRequest, SourceConfigInput,
        SourceFolderResponse, SourceStatus, SyncOutcome,
    },
    domain::{GroupRecord, LibraryFileRecord, LibraryFolderRecord},
    source_store::SourceStore,
};

use super::{
    library::{LibraryService, UpsertNamedTextFileRequest},
    sync::SyncService,
};

pub(crate) const SOURCE_CONFIG_FILENAME: &str = "source.json";
pub(crate) const RECORDS_FOLDER_NAME: &str = "records";

#[derive(Clone)]
pub struct SourceFoldersService {
    library: LibraryService,
    sync: SyncService,
    source_store: SourceStore,
}

#[derive(Debug, Clone)]
struct SourceFolderDescriptor {
    folder: LibraryFolderRecord,
    path: String,
    source_config_file: LibraryFileRecord,
    records_folder: LibraryFolderRecord,
}

impl SourceFoldersService {
    pub fn new(db: crate::db::Database, library: LibraryService, sync: SyncService) -> Self {
        Self {
            library,
            sync,
            source_store: SourceStore::new(db),
        }
    }

    pub async fn migrate_project_sources_in_project(&self, project: &GroupRecord) -> Result<()> {
        let sources = self
            .source_store
            .list_sources_for_group(project.id)
            .await?;
        if sources.is_empty() {
            return Ok(());
        }

        let sources_root = self
            .ensure_folder_in_project(project, None, "sources")
            .await?;
        for source in sources {
            self.ensure_legacy_source_folder(project, &sources_root, &source)
                .await?;
        }
        Ok(())
    }

    pub async fn create_source_folder_in_project(
        &self,
        project: &GroupRecord,
        request: &CreateSourceFolderRequest,
    ) -> Result<SourceFolderResponse> {
        self.sync
            .upsert_source_connection_for_source_folder(&request.source_config)
            .await?;
        let validated = SourceStore::validate_source_input(
            &request.source_config,
            &self.sync.connection_names_for_source_folders().await?,
        )?;

        let folder = self
            .library
            .create_folder_in_project(
                project,
                &CreateFolderRequest {
                    parent_folder_id: request.parent_folder_id,
                    name: request.folder_name.clone(),
                },
            )
            .await?;
        let records_folder = self
            .library
            .create_folder_in_project(
                project,
                &CreateFolderRequest {
                    parent_folder_id: Some(folder.folder_id),
                    name: RECORDS_FOLDER_NAME.to_string(),
                },
            )
            .await?;
        let source_config_file = self
            .library
            .upsert_named_text_file_in_project(
                project,
                &UpsertNamedTextFileRequest {
                    folder_id: Some(folder.folder_id),
                    external_id: source_config_file_external_id(folder.folder_id),
                    filename: SOURCE_CONFIG_FILENAME.to_string(),
                    media_type: "application/json".to_string(),
                    content: serialize_source_config(&request.source_config)?,
                },
            )
            .await?;

        let _ = validated;
        Ok(SourceFolderResponse {
            folder_id: folder.folder_id,
            source_config_file_id: source_config_file
                .files
                .first()
                .map(|file| file.file_id)
                .context("missing source config file")?,
            records_folder_id: records_folder.folder_id,
            path: folder.path,
        })
    }

    pub async fn update_source_folder_config_in_project(
        &self,
        project: &GroupRecord,
        folder_id: Uuid,
        request: &SourceConfigInput,
    ) -> Result<SourceFolderResponse> {
        self.sync
            .upsert_source_connection_for_source_folder(request)
            .await?;
        let _validated = SourceStore::validate_source_input(
            request,
            &self.sync.connection_names_for_source_folders().await?,
        )?;

        let descriptor = self.describe_source_folder(project, folder_id).await?;
        let source_config_file = self
            .library
            .upsert_named_text_file_in_project(
                project,
                &UpsertNamedTextFileRequest {
                    folder_id: Some(folder_id),
                    external_id: source_config_file_external_id(folder_id),
                    filename: SOURCE_CONFIG_FILENAME.to_string(),
                    media_type: "application/json".to_string(),
                    content: serialize_source_config(request)?,
                },
            )
            .await?;

        Ok(SourceFolderResponse {
            folder_id,
            source_config_file_id: source_config_file
                .files
                .first()
                .map(|file| file.file_id)
                .context("missing source config file")?,
            records_folder_id: descriptor.records_folder.id,
            path: descriptor.path,
        })
    }

    pub async fn sync_source_folder_in_project(
        &self,
        project: &GroupRecord,
        folder_id: Uuid,
    ) -> Result<SyncOutcome> {
        let descriptor = self.describe_source_folder(project, folder_id).await?;
        let config_text = self
            .library
            .read_text_file_content(&descriptor.source_config_file)?;
        let input: SourceConfigInput =
            serde_json::from_str(&config_text).context("failed to parse source.json content")?;
        self.sync
            .upsert_source_connection_for_source_folder(&input)
            .await?;
        let validated = SourceStore::validate_source_input(
            &input,
            &self.sync.connection_names_for_source_folders().await?,
        )?;
        self.sync
            .sync_project_source_folder(
                project,
                &descriptor.path,
                folder_id,
                descriptor.records_folder.id,
                &validated,
                &self.library,
            )
            .await
    }

    pub async fn move_source_aware_folder_in_project(
        &self,
        project: &GroupRecord,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<crate::contracts::LibraryFolderResponse> {
        let before = self
            .describe_source_folder_subtree(project, folder_id)
            .await?;
        let old_identities = before
            .iter()
            .map(|descriptor| {
                (
                    descriptor.folder.id,
                    source_folder_identity(project.id, &descriptor.path),
                )
            })
            .collect::<HashMap<_, _>>();

        let moved = self
            .library
            .move_folder_in_project(project, folder_id, request)
            .await?;

        if !old_identities.is_empty() {
            let after = self
                .describe_source_folder_subtree(project, folder_id)
                .await?;
            for descriptor in after {
                if let Some(old_identity) = old_identities.get(&descriptor.folder.id) {
                    let new_identity = source_folder_identity(project.id, &descriptor.path);
                    self.sync
                        .rename_project_source_folder_identity(
                            project.id,
                            old_identity,
                            &new_identity,
                        )
                        .await?;
                }
            }
        }

        Ok(moved)
    }

    pub async fn delete_source_aware_folder_in_project(
        &self,
        project: &GroupRecord,
        folder_id: Uuid,
    ) -> Result<()> {
        let descriptors = self
            .describe_source_folder_subtree(project, folder_id)
            .await?;
        for descriptor in descriptors {
            let identity = source_folder_identity(project.id, &descriptor.path);
            let legacy_source_key = self
                .read_source_config_input(&descriptor)?
                .map(|input| input.source_key);
            self.sync
                .delete_project_source_folder_state(
                    project.id,
                    &identity,
                    legacy_source_key.as_deref(),
                )
                .await?;
        }
        self.library
            .delete_folder_in_project(project, folder_id)
            .await
    }

    async fn ensure_legacy_source_folder(
        &self,
        project: &GroupRecord,
        sources_root: &LibraryFolderRecord,
        source: &SourceStatus,
    ) -> Result<()> {
        let source_folder = self
            .ensure_folder_in_project(project, Some(sources_root.id), &source.source_key)
            .await?;
        let records_folder = self
            .ensure_folder_in_project(project, Some(source_folder.id), RECORDS_FOLDER_NAME)
            .await?;
        let _ = records_folder;

        let content = serialize_source_config(&SourceConfigInput {
            source_key: source.source_key.clone(),
            display_name: if source.display_name == source.source_key {
                None
            } else {
                Some(source.display_name.clone())
            },
            description: source.description.clone(),
            example_queries: source.example_queries.clone(),
            connection: source.connection.clone(),
            database_url: None,
            sync_strategy: source.sync_strategy.clone(),
            connector_type: source.connector_type.clone(),
            base_query: source.base_query.clone(),
            batch_size: source.batch_size,
            visibility: Some(source.visibility),
        })?;

        let files = self.library.list_file_records_in_project(project).await?;
        let already_exists = files.iter().any(|file| {
            file.folder_id == Some(source_folder.id)
                && file.filename.eq_ignore_ascii_case(SOURCE_CONFIG_FILENAME)
        });
        if !already_exists {
            self.library
                .upsert_named_text_file_in_project(
                    project,
                    &UpsertNamedTextFileRequest {
                        folder_id: Some(source_folder.id),
                        external_id: source_config_file_external_id(source_folder.id),
                        filename: SOURCE_CONFIG_FILENAME.to_string(),
                        media_type: "application/json".to_string(),
                        content,
                    },
                )
                .await?;
        }
        Ok(())
    }

    async fn ensure_folder_in_project(
        &self,
        project: &GroupRecord,
        parent_folder_id: Option<Uuid>,
        name: &str,
    ) -> Result<LibraryFolderRecord> {
        let folders = self.library.list_folder_records_in_project(project).await?;
        if let Some(folder) = folders.into_iter().find(|folder| {
            folder.parent_id == parent_folder_id && folder.name.eq_ignore_ascii_case(name)
        }) {
            return Ok(folder);
        }
        let created = self
            .library
            .create_folder_in_project(
                project,
                &CreateFolderRequest {
                    parent_folder_id,
                    name: name.to_string(),
                },
            )
            .await?;
        self.library
            .get_folder_record_in_project(project, created.folder_id)
            .await
    }

    async fn describe_source_folder(
        &self,
        project: &GroupRecord,
        folder_id: Uuid,
    ) -> Result<SourceFolderDescriptor> {
        let folders = self.library.list_folder_records_in_project(project).await?;
        let files = self.library.list_file_records_in_project(project).await?;
        let folder = folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .cloned()
            .with_context(|| format!("unknown folder {folder_id}"))?;
        let path = folder_path_from_records(&folders, folder_id)?;
        build_source_folder_descriptor(&folder, &path, &folders, &files)
    }

    async fn describe_source_folder_subtree(
        &self,
        project: &GroupRecord,
        folder_id: Uuid,
    ) -> Result<Vec<SourceFolderDescriptor>> {
        let folders = self.library.list_folder_records_in_project(project).await?;
        let files = self.library.list_file_records_in_project(project).await?;
        let subtree_ids = descendant_folder_ids(&folders, folder_id);
        let mut descriptors = Vec::new();
        for folder in folders
            .iter()
            .filter(|folder| subtree_ids.contains(&folder.id))
        {
            let path = folder_path_from_records(&folders, folder.id)?;
            if let Ok(descriptor) = build_source_folder_descriptor(folder, &path, &folders, &files)
            {
                descriptors.push(descriptor);
            }
        }
        Ok(descriptors)
    }

    fn read_source_config_input(
        &self,
        descriptor: &SourceFolderDescriptor,
    ) -> Result<Option<SourceConfigInput>> {
        let content = self
            .library
            .read_text_file_content(&descriptor.source_config_file);
        match content {
            Ok(content) => serde_json::from_str(&content)
                .map(Some)
                .context("failed to parse source.json content"),
            Err(error) => Err(error),
        }
    }
}

fn build_source_folder_descriptor(
    folder: &LibraryFolderRecord,
    path: &str,
    folders: &[LibraryFolderRecord],
    files: &[LibraryFileRecord],
) -> Result<SourceFolderDescriptor> {
    let source_config_file = files
        .iter()
        .find(|file| {
            file.folder_id == Some(folder.id)
                && file.filename.eq_ignore_ascii_case(SOURCE_CONFIG_FILENAME)
        })
        .cloned()
        .context("folder is not a source folder")?;
    let records_folder = folders
        .iter()
        .find(|child| child.parent_id == Some(folder.id) && child.name == RECORDS_FOLDER_NAME)
        .cloned()
        .context("source folder is missing records child folder")?;
    Ok(SourceFolderDescriptor {
        folder: folder.clone(),
        path: path.to_string(),
        source_config_file,
        records_folder,
    })
}

fn descendant_folder_ids(folders: &[LibraryFolderRecord], root_folder_id: Uuid) -> HashSet<Uuid> {
    let mut children_by_parent = HashMap::<Option<Uuid>, Vec<Uuid>>::new();
    for folder in folders {
        children_by_parent
            .entry(folder.parent_id)
            .or_default()
            .push(folder.id);
    }

    let mut stack = vec![root_folder_id];
    let mut seen = HashSet::new();
    while let Some(next) = stack.pop() {
        if !seen.insert(next) {
            continue;
        }
        if let Some(children) = children_by_parent.get(&Some(next)) {
            stack.extend(children.iter().copied());
        }
    }
    seen
}

fn folder_path_from_records(folders: &[LibraryFolderRecord], folder_id: Uuid) -> Result<String> {
    let mut by_id = HashMap::new();
    for folder in folders {
        by_id.insert(folder.id, folder.clone());
    }
    let mut current_id = folder_id;
    let mut parts = Vec::new();
    loop {
        let folder = by_id
            .get(&current_id)
            .with_context(|| format!("unknown folder {current_id}"))?;
        parts.push(folder.name.clone());
        match folder.parent_id {
            Some(parent_id) => current_id = parent_id,
            None => break,
        }
    }
    parts.reverse();
    Ok(format!("/{}", parts.join("/")))
}

pub(crate) fn source_folder_identity(group_id: i64, folder_path: &str) -> String {
    format!("group:{group_id}:folder:{}", folder_path.trim())
}

pub(crate) fn source_config_file_external_id(folder_id: Uuid) -> String {
    format!("source-folder:config:{folder_id}")
}

pub(crate) fn source_record_external_id(folder_id: Uuid, external_id: &str) -> String {
    format!("source-folder:record:{folder_id}:{external_id}")
}

pub(crate) fn source_record_filename(external_id: &str) -> String {
    let sanitized = external_id
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other if other.is_control() => '_',
            other => other,
        })
        .collect::<String>();
    let mut stem = sanitized.trim_matches('_').to_string();
    if stem.is_empty() {
        stem = "record".to_string();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    external_id.hash(&mut hasher);
    let digest = format!("{:08x}", hasher.finish());
    format!("{stem}-{digest}.json")
}

pub(crate) fn serialize_source_config(input: &SourceConfigInput) -> Result<String> {
    serde_json::to_string_pretty(input).context("failed to serialize source config")
}

pub(crate) fn serialize_source_record(record: &crate::domain::SourceRecord) -> Result<String> {
    serde_json::to_string_pretty(&json!({
        "external_id": record.external_id,
        "title": record.title,
        "body_text": record.body_text,
        "source_uri": record.source_uri,
        "summary": record.summary,
        "published_at": record.published_at,
        "updated_at": record.updated_at,
        "metadata_json": record.metadata_json,
    }))
    .context("failed to serialize source record")
}
