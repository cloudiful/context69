use anyhow::Result;
use uuid::Uuid;

use super::{FolderRow, LibraryFolderRecord, LibraryStore};
use super::mappers::folder_from_row;

impl LibraryStore {
    pub async fn list_folders(&self) -> Result<Vec<LibraryFolderRecord>> {
        let rows = sqlx::query_as::<_, FolderRow>(
            r#"
            SELECT
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            FROM context69.library_folders
            ORDER BY name, id
            "#,
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(folder_from_row).collect()
    }

    pub async fn get_folder(&self, folder_id: Uuid) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_as::<_, FolderRow>(
            r#"
            SELECT
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            FROM context69.library_folders
            WHERE id = $1
            "#,
        )
        .bind(folder_id)
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn list_folders_in_project(
        &self,
        project_id: i64,
    ) -> Result<Vec<LibraryFolderRecord>> {
        let rows = sqlx::query_as::<_, FolderRow>(
            r#"
            SELECT
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            FROM context69.library_folders
            WHERE project_id = $1
            ORDER BY name, id
            "#,
        )
        .bind(project_id)
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(folder_from_row).collect()
    }

    pub async fn get_folder_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
    ) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_as::<_, FolderRow>(
            r#"
            SELECT
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            FROM context69.library_folders
            WHERE project_id = $1
              AND id = $2
            "#,
        )
        .bind(project_id)
        .bind(folder_id)
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn create_folder(
        &self,
        folder_id: Uuid,
        parent_folder_id: Option<Uuid>,
        name: &str,
    ) -> Result<LibraryFolderRecord> {
        let row = sqlx::query_as::<_, FolderRow>(
            r#"
            WITH parent_scope AS (
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
                SELECT group_id, project_id, visibility FROM parent_scope
                UNION ALL
                SELECT group_id, project_id, visibility FROM default_scope
                LIMIT 1
            )
            INSERT INTO context69.library_folders (id, group_id, project_id, visibility, parent_id, name)
            SELECT $1, rs.group_id, rs.project_id, rs.visibility, $2, $3
            FROM resolved_scope rs
            RETURNING
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            "#,
        )
        .bind(folder_id)
        .bind(parent_folder_id)
        .bind(name)
        .fetch_one(self.db.pool())
        .await?;

        folder_from_row(row)
    }

    pub async fn create_folder_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
        parent_folder_id: Option<Uuid>,
        name: &str,
    ) -> Result<LibraryFolderRecord> {
        let row = sqlx::query_as::<_, FolderRow>(
            r#"
            WITH project_scope AS (
                SELECT p.group_id, p.id AS project_id, p.visibility
                FROM context69.projects p
                WHERE p.id = $4
            ),
            parent_scope AS (
                SELECT group_id, project_id, visibility
                FROM context69.library_folders
                WHERE id = $2 AND project_id = $4
            ),
            resolved_scope AS (
                SELECT group_id, project_id, visibility FROM parent_scope
                UNION ALL
                SELECT group_id, project_id, visibility FROM project_scope
                LIMIT 1
            )
            INSERT INTO context69.library_folders (
                id,
                group_id,
                project_id,
                visibility,
                parent_id,
                name
            )
            SELECT $1, rs.group_id, rs.project_id, rs.visibility, $2, $3
            FROM resolved_scope rs
            RETURNING
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            "#,
        )
        .bind(folder_id)
        .bind(parent_folder_id)
        .bind(name)
        .bind(project_id)
        .fetch_one(self.db.pool())
        .await?;

        folder_from_row(row)
    }

    pub async fn move_folder(
        &self,
        folder_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_as::<_, FolderRow>(
            r#"
            UPDATE context69.library_folders
            SET parent_id = $2, updated_at = now()
            WHERE id = $1
            RETURNING
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            "#,
        )
        .bind(folder_id)
        .bind(target_folder_id)
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn move_folder_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_as::<_, FolderRow>(
            r#"
            UPDATE context69.library_folders
            SET parent_id = $3, updated_at = now()
            WHERE project_id = $1
              AND id = $2
            RETURNING
                group_id,
                (SELECT group_key FROM context69.groups WHERE id = group_id) AS group_key,
                project_id,
                (SELECT project_key FROM context69.projects WHERE id = project_id) AS project_key,
                visibility,
                id,
                parent_id,
                name,
                created_at,
                updated_at
            "#,
        )
        .bind(project_id)
        .bind(folder_id)
        .bind(target_folder_id)
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn delete_folder_record(&self, folder_id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM context69.library_folders WHERE id = $1")
            .bind(folder_id)
            .execute(self.db.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn descendant_folder_ids(&self, folder_id: Uuid) -> Result<Vec<Uuid>> {
        let rows = sqlx::query_scalar::<_, Uuid>(
            r#"
            WITH RECURSIVE descendants AS (
                SELECT id
                FROM context69.library_folders
                WHERE id = $1
                UNION ALL
                SELECT child.id
                FROM context69.library_folders child
                INNER JOIN descendants parent ON child.parent_id = parent.id
            )
            SELECT id FROM descendants
            "#,
        )
        .bind(folder_id)
        .fetch_all(self.db.pool())
        .await?;
        Ok(rows)
    }

    pub async fn descendant_folder_ids_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        let rows = sqlx::query_scalar::<_, Uuid>(
            r#"
            WITH RECURSIVE descendants AS (
                SELECT id
                FROM context69.library_folders
                WHERE project_id = $1
                  AND id = $2
                UNION ALL
                SELECT child.id
                FROM context69.library_folders child
                INNER JOIN descendants parent ON child.parent_id = parent.id
                WHERE child.project_id = $1
            )
            SELECT id FROM descendants
            "#,
        )
        .bind(project_id)
        .bind(folder_id)
        .fetch_all(self.db.pool())
        .await?;
        Ok(rows)
    }
}
