use super::*;
use crate::contracts::Visibility;

impl LibraryService {
    pub async fn list_tree(&self) -> Result<LibraryTreeResponse> {
        let folders = self.store.list_folders().await?;
        let files = self.store.list_files().await?;
        Ok(LibraryTreeResponse {
            root: build_tree(
                "public".to_string(),
                "default-public".to_string(),
                Visibility::Public,
                folders,
                files,
            ),
        })
    }

    pub async fn list_tree_in_project(
        &self,
        project: &crate::domain::ProjectRecord,
    ) -> Result<LibraryTreeResponse> {
        let folders = self.store.list_folders_in_project(project.id).await?;
        let files = self.store.list_files_in_project(project.id).await?;
        Ok(LibraryTreeResponse {
            root: build_tree(
                project.group_key.clone(),
                project.project_key.clone(),
                project.visibility,
                folders,
                files,
            ),
        })
    }

    pub(super) async fn folder_path_by_id(&self, folder_id: Option<Uuid>) -> Result<String> {
        if folder_id.is_none() {
            return Ok("/".to_string());
        }
        let folders = self.store.list_folders().await?;
        let mut by_id = HashMap::new();
        for folder in folders {
            by_id.insert(folder.id, folder);
        }
        build_folder_path(folder_id, &by_id)
    }

    pub(super) async fn folder_path(&self, parent_id: Option<Uuid>, name: &str) -> Result<String> {
        if parent_id.is_none() {
            return Ok(format!("/{}", name));
        }
        let base = self.folder_path_by_id(parent_id).await?;
        Ok(format!("{}/{}", base.trim_end_matches('/'), name))
    }
}

fn build_tree(
    root_group_key: String,
    root_project_key: String,
    root_visibility: Visibility,
    folders: Vec<LibraryFolderRecord>,
    files: Vec<crate::domain::LibraryFileRecord>,
) -> LibraryFolderNode {
    let mut seeds = HashMap::<Option<Uuid>, FolderNodeSeed>::new();
    seeds.insert(
        None,
        FolderNodeSeed {
            folder: None,
            children: Vec::new(),
            files: Vec::new(),
        },
    );

    for folder in &folders {
        seeds.entry(Some(folder.id)).or_insert(FolderNodeSeed {
            folder: Some(folder.clone()),
            children: Vec::new(),
            files: Vec::new(),
        });
        seeds
            .entry(folder.parent_id)
            .or_insert(FolderNodeSeed {
                folder: None,
                children: Vec::new(),
                files: Vec::new(),
            })
            .children
            .push(folder.id);
    }

    for file in files {
        seeds
            .entry(file.folder_id)
            .or_insert(FolderNodeSeed {
                folder: None,
                children: Vec::new(),
                files: Vec::new(),
            })
            .files
            .push(file_to_summary(&file));
    }

    build_folder_node(
        None,
        "/",
        &root_group_key,
        &root_project_key,
        root_visibility,
        &mut seeds,
    )
}

fn build_folder_node(
    folder_id: Option<Uuid>,
    path: &str,
    root_group_key: &str,
    root_project_key: &str,
    root_visibility: Visibility,
    seeds: &mut HashMap<Option<Uuid>, FolderNodeSeed>,
) -> LibraryFolderNode {
    let mut seed = seeds.remove(&folder_id).unwrap_or(FolderNodeSeed {
        folder: None,
        children: Vec::new(),
        files: Vec::new(),
    });

    let mut children = seed
        .children
        .into_iter()
        .filter_map(|child_id| {
            let child = seeds
                .get(&Some(child_id))
                .and_then(|item| item.folder.clone())?;
            let child_path = format!("{}/{}", path.trim_end_matches('/'), child.name);
            Some(build_folder_node(
                Some(child_id),
                &child_path,
                root_group_key,
                root_project_key,
                root_visibility,
                seeds,
            ))
        })
        .collect::<Vec<_>>();
    children.sort_by(|left, right| left.name.cmp(&right.name));
    seed.files
        .sort_by(|left, right| left.filename.cmp(&right.filename));

    let own_processing = seed
        .files
        .iter()
        .filter(|file| {
            matches!(
                file.ingest_status,
                LibraryIngestStatus::Pending | LibraryIngestStatus::Running
            )
        })
        .count();
    let processing_count = own_processing
        + children
            .iter()
            .map(|child| child.processing_count)
            .sum::<usize>();

    match seed.folder {
        Some(folder) => LibraryFolderNode {
            group_key: folder.group_key,
            project_key: folder.project_key,
            visibility: folder.visibility,
            folder_id: Some(folder.id),
            parent_folder_id: folder.parent_id,
            name: folder.name,
            path: path.to_string(),
            processing_count,
            children,
            files: seed.files,
        },
        None => LibraryFolderNode {
            group_key: root_group_key.to_string(),
            project_key: root_project_key.to_string(),
            visibility: root_visibility,
            folder_id: None,
            parent_folder_id: None,
            name: "Root".to_string(),
            path: "/".to_string(),
            processing_count,
            children,
            files: seed.files,
        },
    }
}

fn build_folder_path(
    folder_id: Option<Uuid>,
    folders: &HashMap<Uuid, LibraryFolderRecord>,
) -> Result<String> {
    let Some(mut current_id) = folder_id else {
        return Ok("/".to_string());
    };
    let mut parts = Vec::new();
    loop {
        let folder = folders
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
