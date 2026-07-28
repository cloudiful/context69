use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct StorageMigrationSummary {
    pub scanned: usize,
    pub migrated: usize,
    pub already_migrated: usize,
    pub missing: usize,
    pub invalid: usize,
}

impl LibraryService {
    pub async fn migrate_local_storage_to_active_backend(
        &self,
        dry_run: bool,
    ) -> Result<StorageMigrationSummary> {
        if self.storage.backend() != "s3" {
            return Err(anyhow!("S3 storage must be configured before migration"));
        }
        let mut summary = StorageMigrationSummary::default();
        for file in self.store.list_files().await? {
            summary.scanned += 1;
            if let Some(object) = self
                .store
                .get_storage_object(file.group_id, &file.sha256)
                .await?
                && object.storage_backend == "s3"
                && self.exists_active_storage(&object.object_key).await?
            {
                summary.already_migrated += 1;
                continue;
            }

            let local_path = self.storage_root.join(&file.storage_rel_path);
            let bytes = match fs::read(&local_path) {
                Ok(bytes) => Bytes::from(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    summary.missing += 1;
                    warn!(file_id = %file.id, path = %local_path.display(), "migration source file is missing");
                    continue;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("failed to read {}", local_path.display()));
                }
            };
            if bytes.len() as i64 != file.size_bytes || storage::hash_bytes(&bytes) != file.sha256 {
                summary.invalid += 1;
                warn!(file_id = %file.id, path = %local_path.display(), "migration source file failed size or SHA-256 validation");
                continue;
            }
            if dry_run {
                summary.migrated += 1;
                continue;
            }
            let object = self
                .store_project_content(file.group_id, &file.sha256, bytes)
                .await?;
            self.store
                .link_file_storage_object(file.id, object.id, &object.object_key)
                .await?;
            summary.migrated += 1;
        }
        Ok(summary)
    }
}
