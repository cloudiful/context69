use anyhow::Result;
use chrono::Utc;
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use super::mappers::file_from_row;
use super::{
    FileRow, LibraryFileRecord, LibraryIngestStatus, LibraryStore, NewLibraryFile,
    UpdateLibraryTextFile,
};

const FILE_COLUMNS: &str = r#"
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
    project_id,
    (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
    visibility,
    id,
    folder_id,
    external_id,
    filename,
    media_type,
    size_bytes,
    sha256,
    storage_rel_path,
    ingest_status,
    error_message,
    created_at,
    updated_at,
    ingested_at
"#;

impl LibraryStore {
    pub async fn list_files(&self) -> Result<Vec<LibraryFileRecord>> {
        let query = format!(
            r#"
            SELECT
                {FILE_COLUMNS}
            FROM context69.library_files
            ORDER BY filename, id
            "#
        );
        let rows = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .fetch_all(self.db.pool())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn list_files_by_ids(&self, file_ids: &[Uuid]) -> Result<Vec<LibraryFileRecord>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let query = format!(
            r#"
            SELECT
                {FILE_COLUMNS}
            FROM context69.library_files
            WHERE id = ANY($1)
            ORDER BY filename, id
            "#
        );
        let rows = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(file_ids)
            .fetch_all(self.db.pool())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn list_files_in_project(&self, project_id: i64) -> Result<Vec<LibraryFileRecord>> {
        let query = format!(
            r#"
            SELECT
                {FILE_COLUMNS}
            FROM context69.library_files
            WHERE project_id = $1
            ORDER BY filename, id
            "#
        );
        let rows = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(project_id)
            .fetch_all(self.db.pool())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn get_file(&self, file_id: Uuid) -> Result<Option<LibraryFileRecord>> {
        let query = format!(
            r#"
            SELECT
                {FILE_COLUMNS}
            FROM context69.library_files
            WHERE id = $1
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(file_id)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn get_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<Option<LibraryFileRecord>> {
        let query = format!(
            r#"
            SELECT
                {FILE_COLUMNS}
            FROM context69.library_files
            WHERE project_id = $1
              AND id = $2
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(project_id)
            .bind(file_id)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn get_file_by_external_id_in_project(
        &self,
        project_id: i64,
        external_id: &str,
    ) -> Result<Option<LibraryFileRecord>> {
        let query = format!(
            r#"
            SELECT
                {FILE_COLUMNS}
            FROM context69.library_files
            WHERE project_id = $1
              AND external_id = $2
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(project_id)
            .bind(external_id)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn create_file(&self, file: &NewLibraryFile) -> Result<LibraryFileRecord> {
        let query = format!(
            r#"
            WITH folder_scope AS (
                SELECT group_id, project_id, visibility
                FROM context69.library_folders
                WHERE id = $2
            ),
            default_scope AS (
                SELECT g.id AS group_id, p.id AS project_id, 'public'::text AS visibility
                FROM context69.groups g
                JOIN context69.projects p ON p.group_id = g.id
                WHERE g.group_key = 'public'
                  AND p.project_key = 'default-public'
            ),
            resolved_scope AS (
                SELECT group_id, project_id, visibility FROM folder_scope
                UNION ALL
                SELECT group_id, project_id, visibility FROM default_scope
                LIMIT 1
            )
            INSERT INTO context69.library_files (
                id,
                group_id,
                project_id,
                visibility,
                folder_id,
                external_id,
                filename,
                media_type,
                size_bytes,
                sha256,
                storage_rel_path,
                ingest_status
            )
            SELECT
                $1,
                rs.group_id,
                rs.project_id,
                rs.visibility,
                $2,
                $3,
                $4,
                $5,
                $6,
                $7,
                $8,
                'pending'
            FROM resolved_scope rs
            RETURNING
                {FILE_COLUMNS}
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(file.id)
            .bind(file.folder_id)
            .bind(&file.external_id)
            .bind(&file.filename)
            .bind(&file.media_type)
            .bind(file.size_bytes)
            .bind(&file.sha256)
            .bind(&file.storage_rel_path)
            .fetch_one(self.db.pool())
            .await?;

        file_from_row(row)
    }

    pub async fn create_file_in_project(
        &self,
        project_id: i64,
        file: &NewLibraryFile,
    ) -> Result<LibraryFileRecord> {
        let query = format!(
            r#"
            WITH folder_scope AS (
                SELECT group_id, project_id, visibility
                FROM context69.library_folders
                WHERE id = $2
                  AND project_id = $9
            ),
            project_scope AS (
                SELECT p.group_id, p.id AS project_id, p.visibility
                FROM context69.projects p
                WHERE p.id = $9
            ),
            resolved_scope AS (
                SELECT group_id, project_id, visibility FROM folder_scope
                UNION ALL
                SELECT group_id, project_id, visibility FROM project_scope
                LIMIT 1
            )
            INSERT INTO context69.library_files (
                id,
                group_id,
                project_id,
                visibility,
                folder_id,
                external_id,
                filename,
                media_type,
                size_bytes,
                sha256,
                storage_rel_path,
                ingest_status
            )
            SELECT
                $1,
                rs.group_id,
                rs.project_id,
                rs.visibility,
                $2,
                $3,
                $4,
                $5,
                $6,
                $7,
                $8,
                'pending'
            FROM resolved_scope rs
            RETURNING
                {FILE_COLUMNS}
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(file.id)
            .bind(file.folder_id)
            .bind(&file.external_id)
            .bind(&file.filename)
            .bind(&file.media_type)
            .bind(file.size_bytes)
            .bind(&file.sha256)
            .bind(&file.storage_rel_path)
            .bind(project_id)
            .fetch_one(self.db.pool())
            .await?;

        file_from_row(row)
    }

    pub async fn update_text_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        update: &UpdateLibraryTextFile,
    ) -> Result<Option<LibraryFileRecord>> {
        let query = format!(
            r#"
            UPDATE context69.library_files
            SET
                folder_id = $3,
                external_id = $4,
                filename = $5,
                media_type = $6,
                size_bytes = $7,
                sha256 = $8,
                storage_rel_path = $9,
                ingest_status = 'pending',
                error_message = NULL,
                ingested_at = NULL,
                updated_at = now()
            WHERE project_id = $1
              AND id = $2
            RETURNING
                {FILE_COLUMNS}
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(project_id)
            .bind(file_id)
            .bind(update.folder_id)
            .bind(&update.external_id)
            .bind(&update.filename)
            .bind(&update.media_type)
            .bind(update.size_bytes)
            .bind(&update.sha256)
            .bind(&update.storage_rel_path)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn move_file(
        &self,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        let query = format!(
            r#"
            UPDATE context69.library_files
            SET folder_id = $2, updated_at = now()
            WHERE id = $1
            RETURNING
                {FILE_COLUMNS}
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(file_id)
            .bind(target_folder_id)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn move_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        let query = format!(
            r#"
            UPDATE context69.library_files
            SET folder_id = $3, updated_at = now()
            WHERE project_id = $1
              AND id = $2
            RETURNING
                {FILE_COLUMNS}
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(project_id)
            .bind(file_id)
            .bind(target_folder_id)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn update_file_status(
        &self,
        file_id: Uuid,
        status: LibraryIngestStatus,
        error_message: Option<&str>,
        mark_ingested_now: bool,
    ) -> Result<Option<LibraryFileRecord>> {
        let ingested_at = if mark_ingested_now {
            Some(Utc::now())
        } else {
            None
        };
        let query = format!(
            r#"
            UPDATE context69.library_files
            SET ingest_status = $2, error_message = $3, ingested_at = $4, updated_at = now()
            WHERE id = $1
            RETURNING
                {FILE_COLUMNS}
            "#
        );
        let row = sqlx::query_as::<_, FileRow>(AssertSqlSafe(query))
            .bind(file_id)
            .bind(status.as_str())
            .bind(error_message)
            .bind(ingested_at)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn delete_file_record(&self, file_id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
            .bind(file_id)
            .execute(self.db.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_file_record_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM context69.library_files WHERE project_id = $1 AND id = $2")
                .bind(project_id)
                .bind(file_id)
                .execute(self.db.pool())
                .await?;
        Ok(result.rows_affected() > 0)
    }
}
