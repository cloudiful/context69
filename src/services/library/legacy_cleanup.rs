use crate::library_store::objects::LegacyCleanupRow;

use super::*;

/// Safe default for the legacy cleanup's `--batch-size`; keeps every
/// selection page bounded.
pub const DEFAULT_LEGACY_CLEANUP_BATCH_SIZE: usize = 100;

/// Upper bound for a persisted per-row error message.
const MAX_DELETE_ERROR_LEN: usize = 2000;

/// Counters for one run of the legacy old-key cleanup. Restarting rescans
/// still-open rows, so counters are per invocation, not cumulative.
#[derive(Debug, Clone, Copy, Default)]
pub struct LegacyCleanupSummary {
    pub scanned: usize,
    /// Rows that passed both the backend and live-reference safety checks;
    /// these are the records a destructive run would act on.
    pub eligible: usize,
    pub deleted: usize,
    /// Objects already absent from storage; treated as an idempotent success
    /// because the storage abstraction reports absence explicitly and its
    /// delete is a no-op for absent keys.
    pub already_missing: usize,
    pub skipped_referenced: usize,
    pub skipped_backend: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyCleanupOutcome {
    Eligible,
    Deleted,
    AlreadyMissing,
    SkippedReferenced,
    SkippedBackend,
}

impl LibraryService {
    /// Delete physical objects recorded in
    /// `context69.library_legacy_object_cleanup` once their grace period has
    /// elapsed and no library file references them anymore.
    ///
    /// Safety rules:
    /// - dry runs (`execute = false`) never write to the database and never
    ///   touch storage,
    /// - a record whose backend is unknown (recorded before migration 0024)
    ///   or differs from the active storage backend is skipped, so backend
    ///   switches can never cause wrong-store deletes,
    /// - any live `library_files.storage_rel_path` reference skips deletion,
    /// - the physical delete happens first; only afterwards is the record
    ///   conditionally marked deleted, so a failed DB mark leaves an open row
    ///   whose next run observes the missing object as an idempotent success.
    ///
    /// Restart safety: the id-ordered cursor keeps one permanently failing
    /// row from blocking later pages while remaining deterministic across
    /// restarts; errored rows keep `delete_error` set and stay open.
    pub async fn cleanup_legacy_objects(
        &self,
        execute: bool,
        batch_size: usize,
    ) -> Result<LegacyCleanupSummary> {
        let batch_size = batch_size.max(1);
        let mut summary = LegacyCleanupSummary::default();
        let mut cursor: Option<i64> = None;
        loop {
            let rows = self
                .store
                .list_eligible_legacy_cleanup(cursor, batch_size as i64)
                .await?;
            let last_page = rows.len() < batch_size;
            for row in &rows {
                summary.scanned += 1;
                cursor = Some(row.id);
                match self.cleanup_legacy_row(execute, row).await {
                    Ok(LegacyCleanupOutcome::Eligible) => summary.eligible += 1,
                    Ok(LegacyCleanupOutcome::Deleted) => {
                        summary.eligible += 1;
                        summary.deleted += 1;
                    }
                    Ok(LegacyCleanupOutcome::AlreadyMissing) => {
                        summary.eligible += 1;
                        summary.already_missing += 1;
                    }
                    Ok(LegacyCleanupOutcome::SkippedReferenced) => summary.skipped_referenced += 1,
                    Ok(LegacyCleanupOutcome::SkippedBackend) => summary.skipped_backend += 1,
                    Err(error) => {
                        summary.errors += 1;
                        warn!(
                            cleanup_record_id = row.id,
                            file_id = %row.file_id,
                            old_key = %row.old_key,
                            %error,
                            "legacy object cleanup row failed"
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

    async fn cleanup_legacy_row(
        &self,
        execute: bool,
        row: &LegacyCleanupRow,
    ) -> Result<LegacyCleanupOutcome> {
        // Never delete through a backend the record was not written for. An
        // unknown backend (pre-0024 row) is treated the same as a mismatch:
        // guessing could delete from the wrong store.
        match row.old_storage_backend.as_deref() {
            Some(backend) if backend == self.storage.backend() => {}
            Some(backend) => {
                warn!(
                    cleanup_record_id = row.id,
                    old_key = %row.old_key,
                    recorded_backend = backend,
                    active_backend = self.storage.backend(),
                    "skipping legacy cleanup record recorded on a different storage backend"
                );
                return Ok(LegacyCleanupOutcome::SkippedBackend);
            }
            None => {
                warn!(
                    cleanup_record_id = row.id,
                    old_key = %row.old_key,
                    "skipping legacy cleanup record with unknown storage backend"
                );
                return Ok(LegacyCleanupOutcome::SkippedBackend);
            }
        }

        // Live reference check: any library_files row still pointing at the
        // old key blocks deletion permanently until it moves elsewhere.
        let references = self
            .store
            .count_files_referencing_old_key(&row.old_key)
            .await?;
        if references > 0 {
            info!(
                cleanup_record_id = row.id,
                old_key = %row.old_key,
                references,
                "skipping legacy cleanup record whose old key is still referenced"
            );
            return Ok(LegacyCleanupOutcome::SkippedReferenced);
        }

        if !execute {
            return Ok(LegacyCleanupOutcome::Eligible);
        }

        // All safety checks passed immediately above; nothing runs in between
        // before this irreversible step. Absent objects are an idempotent
        // success: close the open record so it stops being reselected.
        if !self.exists_active_storage(&row.old_key).await? {
            self.store.mark_legacy_cleanup_deleted(row.id).await?;
            info!(
                cleanup_record_id = row.id,
                old_key = %row.old_key,
                "legacy object already absent from storage; cleanup record closed"
            );
            return Ok(LegacyCleanupOutcome::AlreadyMissing);
        }

        if let Err(error) = self.delete_active_storage(&row.old_key).await {
            // Physical delete failed: persist the reason on the still-open
            // row so a later run retries it. Never mark the row deleted here.
            if let Err(mark_error) = self
                .store
                .record_legacy_cleanup_error(row.id, &truncate_error(&error))
                .await
            {
                warn!(
                    cleanup_record_id = row.id,
                    %mark_error,
                    "failed to persist legacy cleanup delete error"
                );
            }
            return Err(error);
        }

        // Physical object is gone; now close the record. A failure here keeps
        // the row open on purpose: the next run sees the object missing and
        // closes it as an idempotent success.
        let marked = self.store.mark_legacy_cleanup_deleted(row.id).await?;
        if !marked {
            // The conditional mark did not land (record already closed); the
            // delete itself still succeeded, so report success.
            warn!(
                cleanup_record_id = row.id,
                old_key = %row.old_key,
                "legacy cleanup record was closed concurrently after physical delete"
            );
        }
        info!(
            cleanup_record_id = row.id,
            group_id = row.group_id,
            file_id = %row.file_id,
            old_key = %row.old_key,
            "deleted legacy direct-path object and closed its cleanup record"
        );
        Ok(LegacyCleanupOutcome::Deleted)
    }
}

/// Cap the persisted error message at `MAX_DELETE_ERROR_LEN` bytes without
/// ever slicing through a multi-byte UTF-8 character: the cut point walks
/// back to the nearest character boundary.
fn truncate_error(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    if text.len() <= MAX_DELETE_ERROR_LEN {
        return text;
    }
    let mut end = MAX_DELETE_ERROR_LEN;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::{MAX_DELETE_ERROR_LEN, truncate_error};

    #[test]
    fn truncation_is_byte_capped_and_never_splits_a_character() {
        // 3-byte characters totalling 2100 bytes so the 2000-byte cut point
        // lands inside a character (2000 % 3 == 2).
        let long = "\u{20ac}".repeat(700);
        assert_eq!(long.len(), 2100);
        let error = anyhow::anyhow!("{long}");
        let truncated = truncate_error(&error);
        assert!(truncated.len() <= MAX_DELETE_ERROR_LEN);
        assert_eq!(
            truncated.chars().count(),
            MAX_DELETE_ERROR_LEN / 3,
            "only complete characters survive"
        );
        assert!(truncated.chars().all(|ch| ch == '\u{20ac}'));

        // Short messages pass through unchanged.
        let short = anyhow::anyhow!("plain failure");
        assert_eq!(truncate_error(&short), "plain failure");
    }
}
