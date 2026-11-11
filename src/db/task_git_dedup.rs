//! Per-repository admission for Git index work (issue #28 phase P4).
//!
//! A repository may have at most one active `git_index` task. Manual sync,
//! scheduler admission, and authenticated webhook work all take the same
//! transaction-scoped advisory lock and then look for an active task, so a
//! second request during a run is reused instead of creating a competing
//! writer. The caller records a durable refresh demand on reuse, which is what
//! turns "already running" into a guaranteed follow-up rather than a dropped
//! event.

use anyhow::Result;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

/// The single repository key a `git_index` submission names, when well-formed.
pub(crate) fn declared_git_repository_key(payloads: &[Value]) -> Option<Uuid> {
    if payloads.len() != 1 {
        return None;
    }
    payloads[0]
        .get("repository_key")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
}

/// Whether one declared `git_index` submission asks for a force rebuild.
///
/// The field is optional and defaults to false; a malformed value is left for
/// the processor's payload validation to reject.
pub(crate) fn declared_git_force_rebuild(payloads: &[Value]) -> bool {
    payloads.len() == 1
        && payloads[0]
            .get("force_rebuild")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// Takes the per-repository admission lock for the caller's transaction.
pub(crate) async fn lock_git_index_admission(
    connection: &mut PgConnection,
    repository_key: Uuid,
) -> Result<()> {
    let key = format!("context69.git_index_admission:{repository_key}");
    sqlx::query_file!("src/sql/db/tasks/lock_git_index_admission.sql", key)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

/// Whether the source still admits Git index work.
pub(crate) async fn git_index_admission_allowed(
    connection: &mut PgConnection,
    group_id: i64,
    repository_key: Uuid,
) -> Result<bool> {
    let row = sqlx::query_file!(
        "src/sql/db/tasks/git_index_admission_allowed.sql",
        group_id,
        repository_key
    )
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.is_some())
}

/// Reads the repository's active `git_index` task, if one exists.
///
/// Callers must hold [`lock_git_index_admission`] in the same transaction.
pub(crate) async fn active_git_index_task(
    connection: &mut PgConnection,
    repository_key: Uuid,
) -> Result<Option<Uuid>> {
    let row = sqlx::query_file!(
        "src/sql/db/tasks/find_active_git_index_task.sql",
        repository_key.to_string()
    )
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.map(|row| row.task_id))
}
