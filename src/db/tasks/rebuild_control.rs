//! The durable projection behind the vector index rebuild controls.
//!
//! The control surface reads what the database records, never what one process
//! happens to remember: the rebuild tasks with the item that carries their
//! rebuild state, and the one active vector index the deployment serves. The
//! rebuild barrier is read by its own reader, so the projection and the
//! barrier describe the same durable state.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::Database;

/// One row of the rebuild control projection.
///
/// The task columns are absent when the deployment never ran a rebuild: the
/// active index is still reported, and there is no task to control.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VectorRebuildControlRow {
    pub active_collection_name: Option<String>,
    pub active_model: Option<String>,
    pub active_dimensions: Option<i64>,
    pub active_rebuilt_chunks: Option<i64>,
    pub id: Option<Uuid>,
    pub status: Option<String>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub error_summary: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub item_id: Option<Uuid>,
    pub item_status: Option<String>,
    pub item_dependency_key: Option<String>,
}

impl Database {
    /// The durable rows of the vector index rebuild control surface.
    ///
    /// The rows are the rebuild tasks, the task that owns the durable rebuild
    /// barrier first, and the one active vector index of the deployment.
    pub async fn read_vector_rebuild_control_rows(&self) -> Result<Vec<VectorRebuildControlRow>> {
        sqlx::query_file_as!(
            VectorRebuildControlRow,
            "src/sql/db/tasks/read_vector_rebuild_control_task.sql"
        )
        .fetch_all(self.pool())
        .await
        .context("read the vector rebuild control rows")
    }
}
