use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use super::mappers::job_from_row;
use super::{JobRow, LibraryIngestJobRecord, LibraryIngestStatus, LibraryStore};

impl LibraryStore {
    pub async fn create_job(&self, job_id: Uuid, file_id: Uuid) -> Result<LibraryIngestJobRecord> {
        let row = sqlx::query_as::<_, JobRow>(
            r#"
            INSERT INTO context69.library_ingest_jobs (
                id,
                group_id,
                visibility,
                file_id,
                status
            )
            SELECT
                $1,
                lf.group_id,
                lf.visibility,
                $2,
                'pending'
            FROM context69.library_files lf
            WHERE lf.id = $2
            RETURNING
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                (SELECT full_path FROM context69.groups WHERE id = group_id) AS group_path,
                visibility,
                id,
                file_id,
                status,
                docling_task_id,
                error_message,
                created_at,
                started_at,
                finished_at,
                updated_at
            "#,
        )
        .bind(job_id)
        .bind(file_id)
        .fetch_one(self.db.pool())
        .await?;

        job_from_row(row)
    }

    pub async fn get_job(&self, job_id: Uuid) -> Result<Option<LibraryIngestJobRecord>> {
        let row = sqlx::query_as::<_, JobRow>(
            r#"
            SELECT
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                (SELECT full_path FROM context69.groups WHERE id = group_id) AS group_path,
                visibility,
                id,
                file_id,
                status,
                docling_task_id,
                error_message,
                created_at,
                started_at,
                finished_at,
                updated_at
            FROM context69.library_ingest_jobs
            WHERE id = $1
            "#,
        )
        .bind(job_id)
        .fetch_optional(self.db.pool())
        .await?;

        row.map(job_from_row).transpose()
    }

    pub async fn get_job_in_project(
        &self,
        project_id: i64,
        job_id: Uuid,
    ) -> Result<Option<LibraryIngestJobRecord>> {
        let row = sqlx::query_as::<_, JobRow>(
            r#"
            SELECT
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                (SELECT full_path FROM context69.groups WHERE id = group_id) AS group_path,
                visibility,
                id,
                file_id,
                status,
                docling_task_id,
                error_message,
                created_at,
                started_at,
                finished_at,
                updated_at
            FROM context69.library_ingest_jobs
            WHERE group_id = $1
              AND id = $2
            "#,
        )
        .bind(project_id)
        .bind(job_id)
        .fetch_optional(self.db.pool())
        .await?;

        row.map(job_from_row).transpose()
    }

    pub async fn list_jobs_for_file(&self, file_id: Uuid) -> Result<Vec<LibraryIngestJobRecord>> {
        let rows = sqlx::query_as::<_, JobRow>(
            r#"
            SELECT
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                (SELECT full_path FROM context69.groups WHERE id = group_id) AS group_path,
                visibility,
                id,
                file_id,
                status,
                docling_task_id,
                error_message,
                created_at,
                started_at,
                finished_at,
                updated_at
            FROM context69.library_ingest_jobs
            WHERE file_id = $1
            ORDER BY created_at DESC, id DESC
            "#,
        )
        .bind(file_id)
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(job_from_row).collect()
    }

    pub async fn update_job_status(
        &self,
        job_id: Uuid,
        status: LibraryIngestStatus,
        docling_task_id: Option<&str>,
        error_message: Option<&str>,
        mark_started_now: bool,
        mark_finished_now: bool,
    ) -> Result<Option<LibraryIngestJobRecord>> {
        let started_at = if mark_started_now {
            Some(Utc::now())
        } else {
            None
        };
        let finished_at = if mark_finished_now {
            Some(Utc::now())
        } else {
            None
        };
        let row = sqlx::query_as::<_, JobRow>(
            r#"
            UPDATE context69.library_ingest_jobs
            SET status = $2,
                docling_task_id = COALESCE($3, docling_task_id),
                error_message = $4,
                started_at = COALESCE($5, started_at),
                finished_at = $6,
                updated_at = now()
            WHERE id = $1
            RETURNING
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                (SELECT full_path FROM context69.groups WHERE id = group_id) AS group_path,
                visibility,
                id,
                file_id,
                status,
                docling_task_id,
                error_message,
                created_at,
                started_at,
                finished_at,
                updated_at
            "#,
        )
        .bind(job_id)
        .bind(status.as_str())
        .bind(docling_task_id)
        .bind(error_message)
        .bind(started_at)
        .bind(finished_at)
        .fetch_optional(self.db.pool())
        .await?;

        row.map(job_from_row).transpose()
    }
}
