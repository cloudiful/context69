//! Best-effort rollback ownership for one failed snapshot attempt.

use uuid::Uuid;

use crate::db::Database;

/// Stable, bounded error code recorded on a failed generation. The concrete
/// failure stays in the returned error, where a caller can classify it.
pub(super) const GENERATION_FAILURE_CODE: &str = "git_index_failed";

/// Tracks how far one attempt got, so a failure cleans up only what exists.
#[derive(Debug, Default)]
pub(super) struct SnapshotProgress {
    /// Configuration revision the attempt was fenced to.
    pub(super) config_revision: Option<i64>,
    /// The source was moved to `indexing`, so a failure should move it back.
    pub(super) indexing_marked: bool,
    /// The building generation to retire and fail, once it exists.
    pub(super) generation_key: Option<Uuid>,
    /// The target changed under the attempt; never touch the source then.
    pub(super) stale: bool,
    /// The generation was activated; a later failure must not roll it back.
    pub(super) activated: bool,
    /// The code destination was recorded before remote writes, so a failure
    /// still owes cleanup of any points that reached the sink.
    pub(super) code_vectors_recorded: bool,
    /// The active generation already matched; nothing new was written.
    pub(super) noop: bool,
}

/// Best-effort rollback of a failed attempt: retire the partial content, mark
/// the building generation failed, and un-stick the source. A stale attempt
/// leaves the source alone because a concurrent change owns it now, and an
/// already-activated generation is never rolled back.
pub(super) async fn fail_snapshot(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    progress: SnapshotProgress,
) {
    if progress.activated {
        return;
    }
    if let Some(generation_key) = progress.generation_key {
        let _ = db
            .replace_git_generation_files(group_id, repository_key, generation_key, &[])
            .await;
        let _ = db
            .fail_git_repository_generation(
                group_id,
                repository_key,
                generation_key,
                GENERATION_FAILURE_CODE,
            )
            .await;
        if progress.code_vectors_recorded {
            let _ = db
                .mark_git_generation_cleanup_pending(
                    group_id,
                    repository_key,
                    generation_key,
                    GENERATION_FAILURE_CODE,
                )
                .await;
        }
    }
    if let (true, false, Some(config_revision)) = (
        progress.indexing_marked,
        progress.stale,
        progress.config_revision,
    ) {
        let _ = db
            .fail_git_repository_index(group_id, repository_key, config_revision)
            .await;
    }
}
