use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use context69_namespace::{
    AccessScope, CreateGroupInput, CreateProjectInput, GroupRecord, MoveProjectInput,
    NamespaceActor, NamespaceMemberRecord, PersonalGroupRecord, ProjectRecord, UpdateGroupInput,
    UpdateProjectInput, UpsertMembershipInput,
};
use sqlx::{AssertSqlSafe, FromRow};

use super::Database;
use crate::{
    contracts::{GroupKind, MembershipRole, Visibility},
    domain::UserRecord,
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
        let group_id = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/create_personal_group.sql",
            &group_key,
            &user.display_name,
            user.id
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/db/namespaces/upsert_personal_group_owner_membership.sql",
            group_id,
            user.id
        )
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
        let row = sqlx::query_file_as!(
            PersonalGroupRow,
            "src/sql/db/namespaces/get_personal_group_for_user.sql",
            user_id
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|row| PersonalGroupRecord {
            group_id: row.group_id,
            group_key: row.group_key,
            role: row.role.parse().unwrap_or(MembershipRole::Owner),
        }))
    }

    pub async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupRecord>> {
        let rows = sqlx::query_file_as!(
            GroupRow,
            "src/sql/db/namespaces/list_groups_for_user.sql",
            user_id
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(group_from_row).collect()
    }

    pub async fn get_group_for_user(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Option<GroupRecord>> {
        let rows = sqlx::query_file_as!(
            GroupRow,
            "src/sql/db/namespaces/get_group_for_user.sql",
            user_id,
            group_key
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().next().map(group_from_row).transpose()
    }

    pub async fn create_group(
        &self,
        actor: &NamespaceActor,
        request: &CreateGroupInput,
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
                .get_group_for_user(actor.user_id, parent_group_key)
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
        let group_id = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/create_group.sql",
            parent_group.as_ref().map(|group| group.id),
            group_key,
            name,
            request.visibility.as_str(),
            kind.as_str(),
            actor.user_id
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/db/namespaces/insert_group_owner_membership.sql",
            group_id,
            actor.user_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.get_group_for_user(actor.user_id, group_key)
            .await?
            .context("created group not found")
    }

    pub async fn update_group(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord> {
        let existing = self
            .get_group_for_user(actor.user_id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Maintainer, "group")?;

        if existing.kind == GroupKind::Personal {
            return Err(anyhow!("personal groups cannot be updated"));
        }

        let next_name = request
            .name
            .as_deref()
            .map(str::trim)
            .unwrap_or(&existing.name);
        if next_name.is_empty() {
            return Err(anyhow!("group name must not be empty"));
        }
        let next_visibility = request.visibility.unwrap_or(existing.visibility);
        if existing.visibility == Visibility::Private
            && next_visibility == Visibility::Public
            && !actor.is_admin
        {
            return Err(anyhow!("only admins can make a group public"));
        }
        if let Some(parent_group_key) = existing.parent_group_key.as_deref() {
            let parent = self
                .get_group_for_user(actor.user_id, parent_group_key)
                .await?
                .context("unknown group")?;
            if parent.visibility == Visibility::Private && next_visibility == Visibility::Public {
                return Err(anyhow!(
                    "group visibility cannot be broader than parent group visibility"
                ));
            }
        }

        sqlx::query_file!(
            "src/sql/db/namespaces/update_group.sql",
            group_key,
            next_name,
            next_visibility.as_str()
        )
        .execute(self.pool())
        .await?;

        self.get_group_for_user(actor.user_id, group_key)
            .await?
            .context("updated group not found")
    }

    pub async fn delete_group(&self, actor: &NamespaceActor, group_key: &str) -> Result<()> {
        let existing = self
            .get_group_for_user(actor.user_id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "group")?;
        if existing.kind == GroupKind::Personal {
            return Err(anyhow!("personal groups cannot be deleted"));
        }
        sqlx::query_file!("src/sql/db/namespaces/delete_group.sql", group_key)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        let group = self
            .get_group_for_user(actor.user_id, group_key)
            .await?
            .context("unknown group")?;
        if group.visibility == Visibility::Private && group.current_role.is_none() {
            return Err(anyhow!("unknown group"));
        }
        let rows = sqlx::query_file_as!(
            MemberRow,
            "src/sql/db/namespaces/list_group_members.sql",
            group_key
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(member_from_row).collect()
    }

    pub async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        let group = self
            .get_group_for_user(actor.user_id, group_key)
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
        sqlx::query_file!(
            "src/sql/db/namespaces/upsert_group_member.sql",
            group.id,
            user.id,
            request.role.as_str()
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        login_name: &str,
    ) -> Result<()> {
        let group = self
            .get_group_for_user(actor.user_id, group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(group.current_role, MembershipRole::Maintainer, "group")?;
        let user = self
            .get_user_by_login_name(login_name)
            .await?
            .context("unknown user")?;
        sqlx::query_file!(
            "src/sql/db/namespaces/delete_group_member.sql",
            group.id,
            user.id
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn list_projects_for_user_in_group(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Vec<ProjectRecord>> {
        let rows = sqlx::query_file_as!(
            ProjectRow,
            "src/sql/db/namespaces/list_projects_for_user_in_group.sql",
            user_id,
            group_key
        )
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
        let rows = sqlx::query_file_as!(
            ProjectRow,
            "src/sql/db/namespaces/get_project_for_user.sql",
            user_id,
            group_key,
            project_key
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().next().map(project_from_row).transpose()
    }

    pub async fn create_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &CreateProjectInput,
    ) -> Result<ProjectRecord> {
        let group = self
            .get_group_for_user(actor.user_id, group_key)
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
        let project_id = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/create_project.sql",
            group.id,
            project_key,
            name,
            request.visibility.as_str(),
            actor.user_id
        )
        .fetch_one(self.pool())
        .await?;
        sqlx::query_file!(
            "src/sql/db/namespaces/insert_project_owner_membership.sql",
            project_id,
            actor.user_id
        )
        .execute(self.pool())
        .await?;
        self.get_project_for_user(actor.user_id, group_key, project_key)
            .await?
            .context("created project not found")
    }

    pub async fn update_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectInput,
    ) -> Result<ProjectRecord> {
        let existing = self
            .get_project_for_user(actor.user_id, group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Maintainer, "project")?;
        let next_name = request
            .name
            .as_deref()
            .map(str::trim)
            .unwrap_or(&existing.name);
        if next_name.is_empty() {
            return Err(anyhow!("project name must not be empty"));
        }
        let next_visibility = request.visibility.unwrap_or(existing.visibility);
        if next_visibility == Visibility::Public && !actor.is_admin {
            return Err(anyhow!("only admins can make a project public"));
        }
        let group = self
            .get_group_for_user(actor.user_id, group_key)
            .await?
            .context("unknown group")?;
        if group.visibility == Visibility::Private && next_visibility == Visibility::Public {
            return Err(anyhow!(
                "project visibility cannot be broader than group visibility"
            ));
        }
        sqlx::query_file!(
            "src/sql/db/namespaces/update_project.sql",
            existing.group_id,
            project_key,
            next_name,
            next_visibility.as_str()
        )
        .execute(self.pool())
        .await?;
        self.get_project_for_user(actor.user_id, group_key, project_key)
            .await?
            .context("updated project not found")
    }

    pub async fn delete_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<()> {
        let existing = self
            .get_project_for_user(actor.user_id, group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "project")?;
        sqlx::query_file!(
            "src/sql/db/namespaces/delete_project.sql",
            existing.group_id,
            project_key
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn list_project_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        let project = self
            .get_project_for_user(actor.user_id, group_key, project_key)
            .await?
            .context("unknown project")?;
        if project.visibility == Visibility::Private && project.current_role.is_none() {
            return Err(anyhow!("unknown project"));
        }
        let rows = sqlx::query_file_as!(
            MemberRow,
            "src/sql/db/namespaces/list_project_members.sql",
            group_key,
            project_key
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(member_from_row).collect()
    }

    pub async fn upsert_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        let project = self
            .get_project_for_user(actor.user_id, group_key, project_key)
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
        sqlx::query_file!(
            "src/sql/db/namespaces/upsert_project_member.sql",
            project.id,
            user.id,
            request.role.as_str()
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<()> {
        let project = self
            .get_project_for_user(actor.user_id, group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(project.current_role, MembershipRole::Maintainer, "project")?;
        let user = self
            .get_user_by_login_name(login_name)
            .await?
            .context("unknown user")?;
        sqlx::query_file!(
            "src/sql/db/namespaces/delete_project_member.sql",
            project.id,
            user.id
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn move_project(
        &self,
        actor: &NamespaceActor,
        source_group_key: &str,
        project_key: &str,
        request: &MoveProjectInput,
    ) -> Result<ProjectRecord> {
        let existing = self
            .get_project_for_user(actor.user_id, source_group_key, project_key)
            .await?
            .context("unknown project")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "project")?;

        let target_group_key = request.target_group_key.trim();
        if target_group_key.is_empty() {
            return Err(anyhow!("target_group_key must not be empty"));
        }

        let target_group = self
            .get_group_for_user(actor.user_id, target_group_key)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(
            target_group.current_role,
            MembershipRole::Maintainer,
            "group",
        )?;

        if target_group.visibility == Visibility::Private
            && existing.visibility == Visibility::Public
        {
            return Err(anyhow!(
                "project visibility cannot be broader than group visibility"
            ));
        }

        let mut tx = self.pool().begin().await?;
        let conflict = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/move_project_conflict.sql",
            target_group.id,
            project_key
        )
        .fetch_optional(&mut *tx)
        .await?;
        if conflict.is_some() {
            tx.rollback().await?;
            return Err(anyhow!("project already exists in target group"));
        }

        sqlx::query_file!(
            "src/sql/db/namespaces/move_project_update_project_group.sql",
            existing.id,
            target_group.id
        )
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
            let query = format!("UPDATE context69.{table} SET group_id = $2 WHERE project_id = $1");
            sqlx::query(AssertSqlSafe(query))
                .bind(existing.id)
                .bind(target_group.id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;

        self.get_project_for_user(actor.user_id, target_group_key, project_key)
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

        let rows = sqlx::query_file_as!(
            PrivateProjectIdRow,
            "src/sql/db/namespaces/resolve_access_scope.sql",
            user_id,
            group_key.as_deref(),
            project_key.as_deref()
        )
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
