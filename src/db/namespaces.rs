use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use super::Database;
use crate::{
    contracts::{
        CreateGroupRequest, CreateProjectRequest, GroupKind, MembershipRole, MoveProjectRequest,
        UpdateGroupRequest, UpdateProjectRequest, UpsertMembershipRequest, Visibility,
    },
    domain::{AccessScope, GroupRecord, NamespaceMemberRecord, PersonalGroupRecord, ProjectRecord, UserRecord},
};

#[derive(Debug, Clone, FromRow)]
struct GroupRow {
    id: i64,
    parent_group_id: Option<i64>,
    parent_group_key: Option<String>,
    group_key: String,
    name: String,
    visibility: String,
    kind: String,
    owner_user_id: Option<i64>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    current_role_rank: Option<i16>,
}

#[derive(Debug, Clone, FromRow)]
struct ProjectRow {
    id: i64,
    group_id: i64,
    group_key: String,
    project_key: String,
    name: String,
    visibility: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    current_role_rank: Option<i16>,
}

#[derive(Debug, Clone, FromRow)]
struct MemberRow {
    user_id: i64,
    login_name: String,
    display_name: String,
    role: String,
}

#[derive(Debug, Clone, FromRow)]
struct PrivateProjectIdRow {
    project_id: i64,
}

#[derive(Debug, Clone, FromRow)]
struct PersonalGroupRow {
    group_id: i64,
    group_key: String,
    role: String,
}

impl Database {
    pub async fn ensure_personal_group_for_user(
        &self,
        user: &UserRecord,
    ) -> Result<PersonalGroupRecord> {
        if let Some(group) = self.get_personal_group_for_user(user.id).await? {
            return Ok(group);
        }

        let group_key = format!("personal-{}", user.login_name);
        let mut tx = self.pool().begin().await?;
        let group_id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO context69.groups (
                group_key,
                name,
                visibility,
                kind,
                owner_user_id,
                created_by_user_id
            )
            VALUES ($1, $2, 'private', 'personal', $3, $3)
            RETURNING id
            "#,
        )
        .bind(&group_key)
        .bind(&user.display_name)
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO context69.group_memberships (group_id, user_id, role)
            VALUES ($1, $2, 'owner')
            ON CONFLICT (group_id, user_id) DO UPDATE
            SET role = EXCLUDED.role,
                updated_at = now()
            "#,
        )
        .bind(group_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(PersonalGroupRecord {
            group_id,
            group_key,
            role: MembershipRole::Owner,
        })
    }

    pub async fn get_personal_group_for_user(
        &self,
        user_id: i64,
    ) -> Result<Option<PersonalGroupRecord>> {
        let row = sqlx::query_as::<_, PersonalGroupRow>(
            r#"
            SELECT
                g.id AS group_id,
                g.group_key,
                gm.role
            FROM context69.groups g
            JOIN context69.group_memberships gm
                ON gm.group_id = g.id AND gm.user_id = $1
            WHERE g.kind = 'personal'
              AND g.owner_user_id = $1
            ORDER BY g.id
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|row| PersonalGroupRecord {
            group_id: row.group_id,
            group_key: row.group_key,
            role: row.role.parse().unwrap_or(MembershipRole::Owner),
        }))
    }

    pub async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupRecord>> {
        let rows = sqlx::query_as::<_, GroupRow>(&group_access_query(None))
            .bind(user_id)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(group_from_row).collect()
    }

    pub async fn get_group_for_user(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Option<GroupRecord>> {
        let rows = sqlx::query_as::<_, GroupRow>(&group_access_query(Some("g.group_key = $2")))
            .bind(user_id)
            .bind(group_key)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().next().map(group_from_row).transpose()
    }

    pub async fn create_group(
        &self,
        actor: &UserRecord,
        request: &CreateGroupRequest,
    ) -> Result<GroupRecord> {
        let group_key = request.group_key.trim();
        let name = request.name.trim();
        if group_key.is_empty() {
            return Err(anyhow!("group_key must not be empty"));
        }
        if name.is_empty() {
            return Err(anyhow!("group name must not be empty"));
        }

        let kind = request.kind.unwrap_or(GroupKind::Shared);
        if kind == GroupKind::Personal {
            return Err(anyhow!("personal groups are created automatically"));
        }
        if request.visibility == Visibility::Public && !actor.is_admin {
            return Err(anyhow!("only admins can create public groups"));
        }

        let parent_group = if let Some(parent_group_key) = request.parent_group_key.as_deref() {
            let group = self
                .get_group_for_user(actor.id, parent_group_key)
                .await?
                .context("unknown group")?;
            ensure_role_at_least(group.current_role, MembershipRole::Maintainer, "group")?;
            if group.visibility == Visibility::Private && request.visibility == Visibility::Public {
                return Err(anyhow!(
                    "child group visibility cannot be broader than parent group visibility"
                ));
            }
            Some(group)
        } else {
            None
        };

        let mut tx = self.pool().begin().await?;
        let group_id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO context69.groups (
                parent_group_id,
                group_key,
                name,
                visibility,
                kind,
                owner_user_id,
                created_by_user_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $6)
            RETURNING id
            "#,
        )
        .bind(parent_group.as_ref().map(|group| group.id))
        .bind(group_key)
        .bind(name)
        .bind(request.visibility.as_str())
        .bind(kind.as_str())
        .bind(actor.id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO context69.group_memberships (group_id, user_id, role)
            VALUES ($1, $2, 'owner')
            "#,
        )
        .bind(group_id)
        .bind(actor.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.get_group_for_user(actor.id, group_key)
            .await?
            .context("created group not found")
    }

    pub async fn update_group(
        &self,
        actor: &UserRecord,
        group_key: &str,
        request: &UpdateGroupRequest,
    ) -> Result<GroupRecord> {
        let existing = self
            .get_group_for_user(actor.id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Maintainer, "group")?;

        if existing.kind == GroupKind::Personal {
            return Err(anyhow!("personal groups cannot be updated"));
        }

        let next_name = request.name.as_deref().map(str::trim).unwrap_or(&existing.name);
        if next_name.is_empty() {
            return Err(anyhow!("group name must not be empty"));
        }
        let next_visibility = request.visibility.unwrap_or(existing.visibility);
        if existing.visibility == Visibility::Private && next_visibility == Visibility::Public && !actor.is_admin {
            return Err(anyhow!("only admins can make a group public"));
        }
        if let Some(parent_group_key) = existing.parent_group_key.as_deref() {
            let parent = self
                .get_group_for_user(actor.id, parent_group_key)
                .await?
                .context("unknown group")?;
            if parent.visibility == Visibility::Private && next_visibility == Visibility::Public {
                return Err(anyhow!(
                    "group visibility cannot be broader than parent group visibility"
                ));
            }
        }

        sqlx::query(
            r#"
            UPDATE context69.groups
            SET name = $2,
                visibility = $3,
                updated_at = now()
            WHERE group_key = $1
            "#,
        )
        .bind(group_key)
        .bind(next_name)
        .bind(next_visibility.as_str())
        .execute(self.pool())
        .await?;

        self.get_group_for_user(actor.id, group_key)
            .await?
            .context("updated group not found")
    }

    pub async fn delete_group(&self, actor: &UserRecord, group_key: &str) -> Result<()> {
        let existing = self
            .get_group_for_user(actor.id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "group")?;
        if existing.kind == GroupKind::Personal {
            return Err(anyhow!("personal groups cannot be deleted"));
        }
        sqlx::query("DELETE FROM context69.groups WHERE group_key = $1")
            .bind(group_key)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn list_group_members(
        &self,
        actor: &UserRecord,
        group_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        let group = self
            .get_group_for_user(actor.id, group_key)
            .await?
            .context("unknown group")?;
        if group.visibility == Visibility::Private && group.current_role.is_none() {
            return Err(anyhow!("unknown group"));
        }
        let rows = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT u.id AS user_id, u.login_name, u.display_name, gm.role
            FROM context69.group_memberships gm
            JOIN context69.users u ON u.id = gm.user_id
            JOIN context69.groups g ON g.id = gm.group_id
            WHERE g.group_key = $1
            ORDER BY u.login_name
            "#,
        )
        .bind(group_key)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(member_from_row).collect()
    }

    pub async fn upsert_group_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()> {
        let group = self
            .get_group_for_user(actor.id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(group.current_role, MembershipRole::Maintainer, "group")?;
        let user = self
            .get_user_by_login_name(request.login_name.trim())
            .await?
            .context("unknown user")?;
        if user.disabled_at.is_some() {
            return Err(anyhow!("user account is disabled"));
        }
        sqlx::query(
            r#"
            INSERT INTO context69.group_memberships (group_id, user_id, role)
            VALUES ($1, $2, $3)
            ON CONFLICT (group_id, user_id) DO UPDATE
            SET role = EXCLUDED.role,
                updated_at = now()
            "#,
        )
        .bind(group.id)
        .bind(user.id)
        .bind(request.role.as_str())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_group_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        login_name: &str,
    ) -> Result<()> {
        let group = self
            .get_group_for_user(actor.id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(group.current_role, MembershipRole::Maintainer, "group")?;
        let user = self
            .get_user_by_login_name(login_name)
            .await?
            .context("unknown user")?;
        sqlx::query(
            r#"
            DELETE FROM context69.group_memberships
            WHERE group_id = $1 AND user_id = $2
            "#,
        )
        .bind(group.id)
        .bind(user.id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn list_projects_for_user_in_group(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Vec<ProjectRecord>> {
        let rows = sqlx::query_as::<_, ProjectRow>(&project_access_query(Some("g.group_key = $2")))
            .bind(user_id)
            .bind(group_key)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(project_from_row).collect()
    }

    pub async fn get_project_for_user(
        &self,
        user_id: i64,
        group_key: &str,
        project_key: &str,
    ) -> Result<Option<ProjectRecord>> {
        let rows = sqlx::query_as::<_, ProjectRow>(&project_access_query(Some(
            "g.group_key = $2 AND p.project_key = $3",
        )))
        .bind(user_id)
        .bind(group_key)
        .bind(project_key)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().next().map(project_from_row).transpose()
    }

    pub async fn create_project(
        &self,
        actor: &UserRecord,
        group_key: &str,
        request: &CreateProjectRequest,
    ) -> Result<ProjectRecord> {
        let group = self
            .get_group_for_user(actor.id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(group.current_role, MembershipRole::Maintainer, "group")?;
        if request.visibility == Visibility::Public && !actor.is_admin {
            return Err(anyhow!("only admins can create public projects"));
        }
        if group.visibility == Visibility::Private && request.visibility == Visibility::Public {
            return Err(anyhow!(
                "project visibility cannot be broader than group visibility"
            ));
        }
        let project_key = request.project_key.trim();
        let name = request.name.trim();
        if project_key.is_empty() {
            return Err(anyhow!("project_key must not be empty"));
        }
        if name.is_empty() {
            return Err(anyhow!("project name must not be empty"));
        }
        let project_id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO context69.projects (
                group_id,
                project_key,
                name,
                visibility,
                created_by_user_id
            )
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#,
        )
        .bind(group.id)
        .bind(project_key)
        .bind(name)
        .bind(request.visibility.as_str())
        .bind(actor.id)
        .fetch_one(self.pool())
        .await?;
        sqlx::query(
            r#"
            INSERT INTO context69.project_memberships (project_id, user_id, role)
            VALUES ($1, $2, 'owner')
            ON CONFLICT (project_id, user_id) DO NOTHING
            "#,
        )
        .bind(project_id)
        .bind(actor.id)
        .execute(self.pool())
        .await?;
        self.get_project_for_user(actor.id, group_key, project_key)
            .await?
            .context("created project not found")
    }

    pub async fn update_project(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectRequest,
    ) -> Result<ProjectRecord> {
        let existing = self
            .get_project_for_user(actor.id, group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Maintainer, "project")?;
        let next_name = request.name.as_deref().map(str::trim).unwrap_or(&existing.name);
        if next_name.is_empty() {
            return Err(anyhow!("project name must not be empty"));
        }
        let next_visibility = request.visibility.unwrap_or(existing.visibility);
        if next_visibility == Visibility::Public && !actor.is_admin {
            return Err(anyhow!("only admins can make a project public"));
        }
        let group = self
            .get_group_for_user(actor.id, group_key)
            .await?
            .context("unknown group")?;
        if group.visibility == Visibility::Private && next_visibility == Visibility::Public {
            return Err(anyhow!(
                "project visibility cannot be broader than group visibility"
            ));
        }
        sqlx::query(
            r#"
            UPDATE context69.projects
            SET name = $3,
                visibility = $4,
                updated_at = now()
            WHERE group_id = $1 AND project_key = $2
            "#,
        )
        .bind(existing.group_id)
        .bind(project_key)
        .bind(next_name)
        .bind(next_visibility.as_str())
        .execute(self.pool())
        .await?;
        self.get_project_for_user(actor.id, group_key, project_key)
            .await?
            .context("updated project not found")
    }

    pub async fn delete_project(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
    ) -> Result<()> {
        let existing = self
            .get_project_for_user(actor.id, group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "project")?;
        sqlx::query(
            r#"
            DELETE FROM context69.projects
            WHERE group_id = $1 AND project_key = $2
            "#,
        )
        .bind(existing.group_id)
        .bind(project_key)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn list_project_members(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        let project = self
            .get_project_for_user(actor.id, group_key, project_key)
            .await?
            .context("unknown project")?;
        if project.visibility == Visibility::Private && project.current_role.is_none() {
            return Err(anyhow!("unknown project"));
        }
        let rows = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT u.id AS user_id, u.login_name, u.display_name, pm.role
            FROM context69.project_memberships pm
            JOIN context69.projects p ON p.id = pm.project_id
            JOIN context69.groups g ON g.id = p.group_id
            JOIN context69.users u ON u.id = pm.user_id
            WHERE g.group_key = $1
              AND p.project_key = $2
            ORDER BY u.login_name
            "#,
        )
        .bind(group_key)
        .bind(project_key)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(member_from_row).collect()
    }

    pub async fn upsert_project_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()> {
        let project = self
            .get_project_for_user(actor.id, group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(project.current_role, MembershipRole::Maintainer, "project")?;
        let user = self
            .get_user_by_login_name(request.login_name.trim())
            .await?
            .context("unknown user")?;
        if user.disabled_at.is_some() {
            return Err(anyhow!("user account is disabled"));
        }
        sqlx::query(
            r#"
            INSERT INTO context69.project_memberships (project_id, user_id, role)
            VALUES ($1, $2, $3)
            ON CONFLICT (project_id, user_id) DO UPDATE
            SET role = EXCLUDED.role,
                updated_at = now()
            "#,
        )
        .bind(project.id)
        .bind(user.id)
        .bind(request.role.as_str())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_project_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<()> {
        let project = self
            .get_project_for_user(actor.id, group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(project.current_role, MembershipRole::Maintainer, "project")?;
        let user = self
            .get_user_by_login_name(login_name)
            .await?
            .context("unknown user")?;
        sqlx::query(
            r#"
            DELETE FROM context69.project_memberships
            WHERE project_id = $1 AND user_id = $2
            "#,
        )
        .bind(project.id)
        .bind(user.id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn move_project(
        &self,
        actor: &UserRecord,
        source_group_key: &str,
        project_key: &str,
        request: &MoveProjectRequest,
    ) -> Result<ProjectRecord> {
        let existing = self
            .get_project_for_user(actor.id, source_group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "project")?;

        let target_group_key = request.target_group_key.trim();
        if target_group_key.is_empty() {
            return Err(anyhow!("target_group_key must not be empty"));
        }

        let target_group = self
            .get_group_for_user(actor.id, target_group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(target_group.current_role, MembershipRole::Maintainer, "group")?;

        if target_group.visibility == Visibility::Private && existing.visibility == Visibility::Public {
            return Err(anyhow!("project visibility cannot be broader than group visibility"));
        }

        let mut tx = self.pool().begin().await?;
        let conflict = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM context69.projects
            WHERE group_id = $1
              AND project_key = $2
            "#,
        )
        .bind(target_group.id)
        .bind(project_key)
        .fetch_optional(&mut *tx)
        .await?;
        if conflict.is_some() {
            tx.rollback().await?;
            return Err(anyhow!("project already exists in target group"));
        }

        sqlx::query(
            r#"
            UPDATE context69.projects
            SET group_id = $2,
                updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(existing.id)
        .bind(target_group.id)
        .execute(&mut *tx)
        .await?;

        for table in [
            "source_configs",
            "source_checkpoints",
            "sync_runs",
            "documents",
            "library_folders",
            "library_files",
            "library_ingest_jobs",
            "library_file_documents",
        ] {
            let query = format!(
                "UPDATE context69.{table} SET group_id = $2 WHERE project_id = $1"
            );
            sqlx::query(&query)
                .bind(existing.id)
                .bind(target_group.id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;

        self.get_project_for_user(actor.id, target_group_key, project_key)
            .await?
            .context("moved project not found")
    }

    pub async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<AccessScope> {
        let Some(user_id) = user_id else {
            return Ok(AccessScope {
                user_id: None,
                include_public: true,
                private_project_ids: Vec::new(),
                group_key,
                project_key,
            });
        };

        let rows = sqlx::query_as::<_, PrivateProjectIdRow>(
            r#"
            WITH RECURSIVE inherited_groups AS (
                SELECT
                    g.id,
                    CASE gm.role
                        WHEN 'owner' THEN 3
                        WHEN 'maintainer' THEN 2
                        ELSE 1
                    END::smallint AS role_rank
                FROM context69.group_memberships gm
                JOIN context69.groups g ON g.id = gm.group_id
                WHERE gm.user_id = $1

                UNION ALL

                SELECT
                    child.id,
                    inherited_groups.role_rank
                FROM context69.groups child
                JOIN inherited_groups ON child.parent_group_id = inherited_groups.id
            ),
            group_roles AS (
                SELECT id AS group_id, MAX(role_rank)::smallint AS role_rank
                FROM inherited_groups
                GROUP BY id
            ),
            project_roles AS (
                SELECT
                    project_id,
                    MAX(
                        CASE role
                            WHEN 'owner' THEN 3
                            WHEN 'maintainer' THEN 2
                            ELSE 1
                        END
                    )::smallint AS role_rank
                FROM context69.project_memberships
                WHERE user_id = $1
                GROUP BY project_id
            )
            SELECT p.id AS project_id
            FROM context69.projects p
            JOIN context69.groups g ON g.id = p.group_id
            LEFT JOIN group_roles gr ON gr.group_id = p.group_id
            LEFT JOIN project_roles pr ON pr.project_id = p.id
            WHERE p.visibility = 'private'
              AND (gr.role_rank IS NOT NULL OR pr.role_rank IS NOT NULL)
              AND ($2::text IS NULL OR g.group_key = $2)
              AND ($3::text IS NULL OR p.project_key = $3)
            ORDER BY p.id
            "#,
        )
        .bind(user_id)
        .bind(group_key.as_deref())
        .bind(project_key.as_deref())
        .fetch_all(self.pool())
        .await?;

        Ok(AccessScope {
            user_id: Some(user_id),
            include_public: true,
            private_project_ids: rows.into_iter().map(|row| row.project_id).collect(),
            group_key,
            project_key,
        })
    }
}

fn group_access_query(extra_filter: Option<&str>) -> String {
    let extra_filter = extra_filter
        .map(|filter| format!(" AND {filter}"))
        .unwrap_or_default();
    format!(
        r#"
        WITH RECURSIVE inherited_groups AS (
            SELECT
                g.id,
                CASE gm.role
                    WHEN 'owner' THEN 3
                    WHEN 'maintainer' THEN 2
                    ELSE 1
                END::smallint AS role_rank
            FROM context69.group_memberships gm
            JOIN context69.groups g ON g.id = gm.group_id
            WHERE gm.user_id = $1

            UNION ALL

            SELECT
                child.id,
                inherited_groups.role_rank
            FROM context69.groups child
            JOIN inherited_groups ON child.parent_group_id = inherited_groups.id
        ),
        group_roles AS (
            SELECT id AS group_id, MAX(role_rank)::smallint AS role_rank
            FROM inherited_groups
            GROUP BY id
        )
        SELECT
            g.id,
            g.parent_group_id,
            parent.group_key AS parent_group_key,
            g.group_key,
            g.name,
            g.visibility,
            g.kind,
            g.owner_user_id,
            g.created_at,
            g.updated_at,
            gr.role_rank AS current_role_rank
        FROM context69.groups g
        LEFT JOIN context69.groups parent ON parent.id = g.parent_group_id
        LEFT JOIN group_roles gr ON gr.group_id = g.id
        WHERE (g.visibility = 'public' OR gr.role_rank IS NOT NULL){extra_filter}
        ORDER BY g.group_key
        "#
    )
}

fn project_access_query(extra_filter: Option<&str>) -> String {
    let extra_filter = extra_filter
        .map(|filter| format!(" AND {filter}"))
        .unwrap_or_default();
    format!(
        r#"
        WITH RECURSIVE inherited_groups AS (
            SELECT
                g.id,
                CASE gm.role
                    WHEN 'owner' THEN 3
                    WHEN 'maintainer' THEN 2
                    ELSE 1
                END::smallint AS role_rank
            FROM context69.group_memberships gm
            JOIN context69.groups g ON g.id = gm.group_id
            WHERE gm.user_id = $1

            UNION ALL

            SELECT
                child.id,
                inherited_groups.role_rank
            FROM context69.groups child
            JOIN inherited_groups ON child.parent_group_id = inherited_groups.id
        ),
        group_roles AS (
            SELECT id AS group_id, MAX(role_rank)::smallint AS role_rank
            FROM inherited_groups
            GROUP BY id
        ),
        project_roles AS (
            SELECT
                project_id,
                MAX(
                    CASE role
                        WHEN 'owner' THEN 3
                        WHEN 'maintainer' THEN 2
                        ELSE 1
                    END
                )::smallint AS role_rank
            FROM context69.project_memberships
            WHERE user_id = $1
            GROUP BY project_id
        )
        SELECT
            p.id,
            p.group_id,
            g.group_key,
            p.project_key,
            p.name,
            p.visibility,
            p.created_at,
            p.updated_at,
            GREATEST(COALESCE(gr.role_rank, 0), COALESCE(pr.role_rank, 0))::smallint
                AS current_role_rank
        FROM context69.projects p
        JOIN context69.groups g ON g.id = p.group_id
        LEFT JOIN group_roles gr ON gr.group_id = p.group_id
        LEFT JOIN project_roles pr ON pr.project_id = p.id
        WHERE (
            p.visibility = 'public'
            OR gr.role_rank IS NOT NULL
            OR pr.role_rank IS NOT NULL
        ){extra_filter}
        ORDER BY g.group_key, p.project_key
        "#
    )
}

fn ensure_role_at_least(
    actual: Option<MembershipRole>,
    required: MembershipRole,
    resource_name: &str,
) -> Result<()> {
    let actual = actual.ok_or_else(|| anyhow!("insufficient permissions for {resource_name}"))?;
    if actual.rank() < required.rank() {
        return Err(anyhow!("insufficient permissions for {resource_name}"));
    }
    Ok(())
}

fn group_from_row(row: GroupRow) -> Result<GroupRecord> {
    Ok(GroupRecord {
        id: row.id,
        parent_group_id: row.parent_group_id,
        parent_group_key: row.parent_group_key,
        group_key: row.group_key,
        name: row.name,
        visibility: row.visibility.parse()?,
        kind: row.kind.parse()?,
        owner_user_id: row.owner_user_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
        current_role: role_from_rank(row.current_role_rank),
    })
}

fn project_from_row(row: ProjectRow) -> Result<ProjectRecord> {
    Ok(ProjectRecord {
        id: row.id,
        group_id: row.group_id,
        group_key: row.group_key,
        project_key: row.project_key,
        name: row.name,
        visibility: row.visibility.parse()?,
        created_at: row.created_at,
        updated_at: row.updated_at,
        current_role: role_from_rank(row.current_role_rank),
    })
}

fn member_from_row(row: MemberRow) -> Result<NamespaceMemberRecord> {
    Ok(NamespaceMemberRecord {
        user_id: row.user_id,
        login_name: row.login_name,
        display_name: row.display_name,
        role: row.role.parse()?,
    })
}

fn role_from_rank(rank: Option<i16>) -> Option<MembershipRole> {
    match rank {
        Some(3) => Some(MembershipRole::Owner),
        Some(2) => Some(MembershipRole::Maintainer),
        Some(1) => Some(MembershipRole::Viewer),
        _ => None,
    }
}
