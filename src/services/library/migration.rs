use chrono::{DateTime, Duration as ChronoDuration, Utc};

use crate::library_store::LegacyDirectPathFileRow;

use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct StorageMigrationSummary {
    pub scanned: usize,
    pub migrated: usize,
    pub already_migrated: usize,
    pub missing: usize,
    pub invalid: usize,
}

/// Grace period before a recorded legacy old key becomes eligible for the
/// separate cleanup phase. Old-key deletion never runs in this tool.
const LEGACY_CLEANUP_GRACE_DAYS: i64 = 7;

/// Safe default for the legacy migration's `--batch-size`; keeps every
/// selection page bounded.
pub const DEFAULT_LEGACY_PATH_MIGRATION_BATCH_SIZE: usize = 100;

/// Counters for one run of the legacy direct-path migration. Restarting
/// rescans unmigrated rows, so counters are per invocation, not cumulative.
#[derive(Debug, Clone, Copy, Default)]
pub struct LegacyPathMigrationSummary {
    pub scanned: usize,
    pub migrated: usize,
    pub already_migrated: usize,
    pub missing: usize,
    pub invalid: usize,
    /// The row changed concurrently without being linked elsewhere; its new
    /// object was cleaned up only when unreferenced.
    pub conflicts: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyRowOutcome {
    Migrated,
    AlreadyMigrated,
    Missing,
    Invalid,
    Conflict,
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

    /// Migrate legacy UUID direct-path rows (`storage_object_id IS NULL`) into
    /// the existing content-addressed layout. Every source object is read back
    /// through the active storage abstraction and verified against its stored
    /// size and SHA-256 before anything is written. The reference update and
    /// the durable old-key cleanup record commit together; old keys themselves
    /// are never deleted by this phase.
    ///
    /// Restart safety: linked rows leave the selection set, and the
    /// `(created_at, id)` cursor keeps one permanently failing row from
    /// blocking later rows within an invocation while remaining deterministic
    /// across restarts. Per-row failures are counted and reported; only a
    /// fatal selection error aborts the run. Dry runs perform no database or
    /// storage writes.
    pub async fn migrate_legacy_direct_paths(
        &self,
        dry_run: bool,
        batch_size: usize,
    ) -> Result<LegacyPathMigrationSummary> {
        let batch_size = batch_size.max(1);
        let mut summary = LegacyPathMigrationSummary::default();
        let mut cursor: Option<(DateTime<Utc>, Uuid)> = None;
        loop {
            let (after_created_at, after_id) = match cursor {
                Some((created_at, id)) => (Some(created_at), Some(id)),
                None => (None, None),
            };
            let rows = self
                .store
                .list_legacy_direct_path_files(after_created_at, after_id, batch_size as i64)
                .await?;
            let last_page = rows.len() < batch_size;
            for row in &rows {
                summary.scanned += 1;
                cursor = Some((row.created_at, row.id));
                match self.migrate_legacy_direct_path_row(dry_run, row).await {
                    Ok(LegacyRowOutcome::Migrated) => summary.migrated += 1,
                    Ok(LegacyRowOutcome::AlreadyMigrated) => summary.already_migrated += 1,
                    Ok(LegacyRowOutcome::Missing) => summary.missing += 1,
                    Ok(LegacyRowOutcome::Invalid) => summary.invalid += 1,
                    Ok(LegacyRowOutcome::Conflict) => summary.conflicts += 1,
                    Err(error) => {
                        summary.errors += 1;
                        warn!(
                            file_id = %row.id,
                            old_key = %row.storage_rel_path,
                            %error,
                            "legacy direct-path migration row failed"
                        );
                    }
                }
            }
            if last_page || rows.is_empty() {
                break;
            }
        }
        Ok(summary)
    }

    async fn migrate_legacy_direct_path_row(
        &self,
        dry_run: bool,
        row: &LegacyDirectPathFileRow,
    ) -> Result<LegacyRowOutcome> {
        // Never trust the database path alone: read it back and verify before
        // touching the content-addressed layout or any database state.
        let bytes = match self.read_active_storage(&row.storage_rel_path).await? {
            Some(bytes) => bytes,
            None => return Ok(LegacyRowOutcome::Missing),
        };
        if bytes.len() as i64 != row.size_bytes || storage::hash_bytes(&bytes) != row.sha256 {
            warn!(
                file_id = %row.id,
                old_key = %row.storage_rel_path,
                "legacy direct-path source failed size or SHA-256 validation"
            );
            return Ok(LegacyRowOutcome::Invalid);
        }

        if dry_run {
            return Ok(LegacyRowOutcome::Migrated);
        }

        let key = object_storage::content_object_key(row.group_id, &row.sha256);
        let existing = self
            .store
            .get_storage_object(row.group_id, &row.sha256)
            .await?;
        let reusable = match existing.as_ref() {
            Some(object)
                if object.storage_backend == self.storage.backend()
                    && object.object_key == key
                    && object.size_bytes == bytes.len() as i64
                    && object.staging_lease_until.is_none() =>
            {
                self.exists_active_storage(&object.object_key).await?
            }
            _ => false,
        };
        let object = if reusable {
            existing.expect("reusable object was just loaded")
        } else {
            self.store_project_content(row.group_id, &row.sha256, bytes.clone())
                .await?
        };

        // Reference update and durable old-key record must commit together.
        // On any failure the newly created object must not leak: the guarded
        // delete below no-ops whenever the object ended up referenced (e.g.
        // the link actually committed despite the reported error).
        match self.commit_legacy_reference(row, &object).await {
            Ok(Some(())) => {
                info!(
                    file_id = %row.id,
                    group_id = row.group_id,
                    old_key = %row.storage_rel_path,
                    object_key = %object.object_key,
                    storage_object_id = %object.id,
                    "migrated legacy direct-path library file onto content-addressed storage"
                );
                Ok(LegacyRowOutcome::Migrated)
            }
            Ok(None) => {
                // The conditional update did not land: classify why and drop
                // an object this run created while it is still unreferenced.
                if !reusable {
                    self.discard_created_object(row.id, object.id).await;
                }
                Ok(self.classify_conditional_failure(row.id).await)
            }
            Err(error) => {
                if !reusable {
                    // Only objects this invocation created may be discarded;
                    // a pre-existing unreferenced object must be left alone.
                    self.discard_created_object(row.id, object.id).await;
                }
                Err(error)
            }
        }
    }

    /// Commit the conditional legacy reference update together with the
    /// durable old-key record. Returns `Ok(None)` when the conditional update
    /// did not land because the row changed concurrently; `Ok(Some(()))` on a
    /// successful joint commit.
    async fn commit_legacy_reference(
        &self,
        row: &LegacyDirectPathFileRow,
        object: &crate::library_store::objects::StorageObjectRecord,
    ) -> Result<Option<()>> {
        let mut tx = self.db.pool().begin().await?;
        let updated = self
            .store
            .link_legacy_file_storage_object_on_connection(
                &mut tx,
                row.id,
                &row.storage_rel_path,
                object.id,
                &object.object_key,
            )
            .await?;
        if !updated {
            tx.rollback().await?;
            return Ok(None);
        }
        self.store
            .record_legacy_object_cleanup_on_connection(
                &mut tx,
                row.group_id,
                row.id,
                &row.storage_rel_path,
                Utc::now() + ChronoDuration::days(LEGACY_CLEANUP_GRACE_DAYS),
            )
            .await?;
        tx.commit().await?;
        Ok(Some(()))
    }

    /// Best-effort removal of an object this run created after its reference
    /// update failed or did not land. The guarded store helper never deletes a
    /// referenced or shared object; afterwards the outcome is verified so a
    /// silent miss (for example a physical-delete failure) is observable.
    async fn discard_created_object(&self, file_id: Uuid, object_id: Uuid) {
        self.delete_unreferenced_storage_object(object_id).await;
        let references = match self.store.count_storage_object_references(object_id).await {
            Ok(references) => references,
            Err(error) => {
                warn!(
                    file_id = %file_id,
                    object_id = %object_id,
                    %error,
                    "failed to verify storage object reference count after guarded cleanup"
                );
                return;
            }
        };
        if references > 0 {
            // The object is legitimately shared now; nothing to report.
            return;
        }
        if let Ok(Some(object)) = self.store.get_storage_object_by_id(object_id).await {
            warn!(
                file_id = %file_id,
                object_id = %object_id,
                object_key = %object.object_key,
                "newly created legacy migration storage object survived guarded cleanup"
            );
        }
    }

    async fn classify_conditional_failure(&self, file_id: Uuid) -> LegacyRowOutcome {
        match self.store.get_legacy_file_storage_state(file_id).await {
            Ok(Some(state)) if state.storage_object_id.is_some() => {
                LegacyRowOutcome::AlreadyMigrated
            }
            Ok(_) => LegacyRowOutcome::Conflict,
            Err(error) => {
                warn!(
                    file_id = %file_id,
                    %error,
                    "failed to classify legacy reference update conflict"
                );
                LegacyRowOutcome::Conflict
            }
        }
    }
}
