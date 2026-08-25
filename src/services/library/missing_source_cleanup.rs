use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::{GroupKind, LibraryIngestStatus, Visibility};
use crate::library_store::MissingLegacySourceFileRow;

use super::*;

/// Grace period before a confirmed-missing legacy direct-path source becomes
/// eligible for startup cleanup. Conservative so transient S3 / filesystem
/// races during upload, plus any late migration writes, are not destructive.
/// The selection query also requires the row to be in a terminal ingest
/// state, so this grace only delays the missing-source check itself.
pub const MISSING_SOURCE_CLEANUP_GRACE_HOURS: i64 = 24;

/// Safe default for the startup cleanup's selection page; keeps every
/// selection batch bounded.
pub const DEFAULT_MISSING_SOURCE_CLEANUP_BATCH_SIZE: usize = 100;

/// Counters for one run of the startup missing-source cleanup. Counters are
/// per invocation, not cumulative: restarting rescans candidates whose row
/// state or storage key has changed since the previous attempt.
#[derive(Debug, Clone, Copy, Default)]
pub struct MissingSourceCleanupSummary {
    pub scanned: usize,
    pub confirmed_missing: usize,
    pub deleted: usize,
    pub still_present: usize,
    pub skipped_recent_nonterminal: usize,
    pub errors: usize,
    /// Qdrant/vector runtime was unavailable, so the whole cleanup was
    /// skipped; the next startup retries with the same grace window. Never
    /// set together with `scanned` because the selection loop never runs.
    pub qdrant_unavailable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingSourceRowOutcome {
    /// Source confirmed missing; the row was deleted together with its
    /// derived PostgreSQL and Qdrant data.
    Deleted,
    /// Source is present in the active storage backend; nothing to do.
    StillPresent,
    /// Row was concurrently linked, re-ingested, deleted, or otherwise no
    /// longer matches the missing-source criteria.
    SkippedRecentNonterminal,
}

impl LibraryService {
    /// Run the missing-source cleanup as part of normal application startup,
    /// after the legacy UUID direct-path migration has run and before
    /// pending task workers resume.
    ///
    /// Per-row work is bounded by [`DEFAULT_MISSING_SOURCE_CLEANUP_BATCH_SIZE`]
    /// and driven by the same `(created_at, id)` cursor the migration uses.
    /// Failures are isolated: a single row that errors or fails to verify
    /// does not abort the rest of the batch. Qdrant availability is gated
    /// before any candidate is touched, so a Qdrant outage never strands
    /// PostgreSQL rows without their vector points.
    pub async fn run_startup_missing_source_cleanup(&self) -> Result<MissingSourceCleanupSummary> {
        self.clean_missing_legacy_sources(
            MISSING_SOURCE_CLEANUP_GRACE_HOURS,
            DEFAULT_MISSING_SOURCE_CLEANUP_BATCH_SIZE,
        )
        .await
    }

    /// Parameterized missing-source cleanup. The startup wrapper pins
    /// [`MISSING_SOURCE_CLEANUP_GRACE_HOURS`]; tests use a shorter grace to
    /// exercise the path without waiting a day per scenario.
    pub async fn clean_missing_legacy_sources(
        &self,
        grace_hours: i64,
        batch_size: usize,
    ) -> Result<MissingSourceCleanupSummary> {
        let batch_size = batch_size.max(1);
        let mut summary = MissingSourceCleanupSummary::default();
        if self.runtime.is_none() {
            // Qdrant (and therefore the vector delete path) is unavailable.
            // Skip the whole cleanup safely; the next startup retries with
            // the same grace window.
            summary.qdrant_unavailable = true;
            warn!(
                "skipping missing-source cleanup: qdrant/vector runtime is unavailable; \
                 it will retry on the next startup"
            );
            return Ok(summary);
        }

        let mut cursor: Option<(DateTime<Utc>, Uuid)> = None;
        loop {
            let (after_created_at, after_id) = match cursor {
                Some((created_at, id)) => (Some(created_at), Some(id)),
                None => (None, None),
            };
            let rows = self
                .store
                .list_missing_legacy_source_files(
                    grace_hours,
                    after_created_at,
                    after_id,
                    batch_size as i64,
                )
                .await?;
            let last_page = rows.len() < batch_size;
            for row in &rows {
                summary.scanned += 1;
                cursor = Some((row.created_at, row.id));
                match self.clean_missing_source_row(row).await {
                    Ok(MissingSourceRowOutcome::Deleted) => {
                        summary.confirmed_missing += 1;
                        summary.deleted += 1;
                    }
                    Ok(MissingSourceRowOutcome::StillPresent) => {
                        summary.still_present += 1;
                    }
                    Ok(MissingSourceRowOutcome::SkippedRecentNonterminal) => {
                        summary.skipped_recent_nonterminal += 1;
                    }
                    Err(error) => {
                        summary.errors += 1;
                        warn!(
                            file_id = %row.id,
                            old_key = %row.storage_rel_path,
                            %error,
                            "missing-source cleanup row failed; will retry on the next startup"
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

    async fn clean_missing_source_row(
        &self,
        row: &MissingLegacySourceFileRow,
    ) -> Result<MissingSourceRowOutcome> {
        // Narrow guard: `delete_file_in_project` calls
        // `delete_file_ids`, which silently skips the Qdrant delete when
        // the runtime is None. The cleanup establishes the runtime is
        // present at entry to `clean_missing_legacy_sources`; this local
        // assertion makes that invariant explicit and surfaces a Qdrant
        // outage as a failure before we touch the database row, instead
        // of letting the chain silently strand vector points.
        if self.runtime.is_none() {
            anyhow::bail!(
                "missing-source cleanup requires the qdrant runtime; \
                 the chain silently skips qdrant without it"
            );
        }

        // Re-read the file row to guard against a concurrent change since
        // the selection page was built. The migration runs first and the
        // service hasn't started task workers yet, but a defensive re-read
        // is cheap and prevents the cleanup from racing a late migration
        // write. We need the storage-object linkage, the resolved path, and
        // the current ingest status in three independent fields.
        let storage_paths = self.store.list_storage_paths_for_files(&[row.id]).await?;
        let Some(path) = storage_paths.into_iter().next() else {
            return Ok(MissingSourceRowOutcome::SkippedRecentNonterminal);
        };
        if path.storage_object_id.is_some() {
            // Concurrently linked onto the content-addressed layout.
            return Ok(MissingSourceRowOutcome::SkippedRecentNonterminal);
        }
        if path.storage_rel_path != row.storage_rel_path {
            // Storage path was rewritten concurrently.
            return Ok(MissingSourceRowOutcome::SkippedRecentNonterminal);
        }
        let Some(file) = self.store.get_file(row.id).await? else {
            return Ok(MissingSourceRowOutcome::SkippedRecentNonterminal);
        };
        if file.group_id != row.group_id {
            // Group reassignment is not a normal operation, but be safe.
            return Ok(MissingSourceRowOutcome::SkippedRecentNonterminal);
        }
        if !matches!(
            file.ingest_status,
            LibraryIngestStatus::Succeeded | LibraryIngestStatus::Failed
        ) {
            // Non-terminal: pending, running, or cancelled.
            return Ok(MissingSourceRowOutcome::SkippedRecentNonterminal);
        }

        // Confirm the source is genuinely missing: storage errors do not
        // qualify, only an explicit NotFound from the storage abstraction.
        let exists = match self.exists_active_storage(&path.storage_rel_path).await {
            Ok(exists) => exists,
            Err(error) => {
                warn!(
                    file_id = %row.id,
                    old_key = %path.storage_rel_path,
                    %error,
                    "missing-source cleanup storage check failed; will retry on the next startup"
                );
                return Err(error);
            }
        };
        if exists {
            return Ok(MissingSourceRowOutcome::StillPresent);
        }

        // Build a minimal project record for the existing
        // `delete_file_in_project` chain. Only `project.id` is consulted by
        // that path; the remaining fields are unused placeholders.
        let project = crate::domain::GroupRecord {
            id: file.group_id,
            parent_group_id: None,
            group_path: String::new(),
            parent_group_path: None,
            group_key: String::new(),
            name: String::new(),
            visibility: Visibility::Public,
            kind: GroupKind::Shared,
            owner_user_id: None,
            created_at: file.created_at,
            updated_at: file.updated_at,
            current_role: None,
        };

        // `delete_file_in_project` deletes Qdrant points (chunk-id based) and
        // the documents before the library_files row, so a Qdrant failure
        // stops the chain before the row goes away. The runtime was checked
        // at the entry of this run; the helper itself is the known runtime
        // path and never silently skips Qdrant.
        self.delete_file_in_project(&project, file.id).await?;

        info!(
            file_id = %file.id,
            group_id = file.group_id,
            old_key = %path.storage_rel_path,
            "deleted missing-source legacy direct-path file and its derived data"
        );
        Ok(MissingSourceRowOutcome::Deleted)
    }
}
