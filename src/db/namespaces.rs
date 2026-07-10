use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use context69_namespace::{
    AccessScope, CreateGroupInput, GroupRecord, MoveGroupInput, NamespaceActor,
    NamespaceMemberRecord, PersonalGroupRecord, UpdateGroupInput, UpsertMembershipInput,
};
use sqlx::FromRow;

use super::Database;
use crate::{
    contracts::{GroupKind, MembershipRole, Visibility},
    domain::UserRecord,
};

#[derive(Debug, Clone, FromRow)]
struct GroupRow {
    id: i64,
    parent_group_id: Option<i64>,
    group_path: String,
    parent_group_path: Option<String>,
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
struct MemberRow {
    user_id: i64,
    login_name: String,
    display_name: String,
    role: String,
}

#[derive(Debug, Clone, FromRow)]
struct PrivateGroupIdRow {
    group_id: i64,
}

#[derive(Debug, Clone, FromRow)]
struct PersonalGroupRow {
    group_id: i64,
    group_path: String,
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
            group_path: group_key,
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
            group_path: row.group_path,
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
        group_path: &str,
    ) -> Result<Option<GroupRecord>> {
        let rows = sqlx::query_file_as!(
            GroupRow,
            "src/sql/db/namespaces/get_group_for_user.sql",
            user_id,
            group_path
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

        let parent_group = if let Some(parent_group_path) = request.parent_group_path.as_deref() {
            let group = self
                .get_group_for_user(actor.user_id, parent_group_path)
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

        let group_path = if let Some(parent_group) = parent_group {
            format!("{}/{}", parent_group.group_path, group_key)
        } else {
            group_key.to_string()
        };

        self.get_group_for_user(actor.user_id, &group_path)
            .await?
            .context("created group not found")
    }

    pub async fn update_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord> {
        let existing = self
            .get_group_for_user(actor.user_id, group_path)
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
        if let Some(parent_group_path) = existing.parent_group_path.as_deref() {
            let parent = self
                .get_group_for_user(actor.user_id, parent_group_path)
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
            group_path,
            next_name,
            next_visibility.as_str()
        )
        .execute(self.pool())
        .await?;

        self.get_group_for_user(actor.user_id, group_path)
            .await?
            .context("updated group not found")
    }

    pub async fn move_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &MoveGroupInput,
    ) -> Result<GroupRecord> {
        let existing = self
            .get_group_for_user(actor.user_id, group_path)
            .await?
            .context("unknown group")?;

        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "group")?;
        if existing.kind == GroupKind::Personal {
            return Err(anyhow!("personal groups cannot be moved"));
        }

        let target_parent = if let Some(target_parent_path) = request.target_parent_group_path.as_deref()
        {
            let target = self
                .get_group_for_user(actor.user_id, target_parent_path)
                .await?
                .context("unknown group")?;
            ensure_role_at_least(target.current_role, MembershipRole::Maintainer, "group")?;
            if target.group_path == existing.group_path
                || target
                    .group_path
                    .starts_with(&format!("{}/", existing.group_path))
            {
                return Err(anyhow!("group cannot be moved into its descendant"));
            }
            if target.visibility == Visibility::Private && existing.visibility == Visibility::Public {
                return Err(anyhow!(
                    "group visibility cannot be broader than parent group visibility"
                ));
            }
            Some(target)
        } else {
            None
        };

        sqlx::query_file!(
            "src/sql/db/namespaces/move_group_update_parent.sql",
            existing.id,
            target_parent.as_ref().map(|group| group.id)
        )
        .execute(self.pool())
        .await?;
        sqlx::query_file!("src/sql/db/namespaces/rebuild_group_paths.sql")
            .execute(self.pool())
            .await?;

        let moved_path = if let Some(target_parent) = target_parent {
            format!("{}/{}", target_parent.group_path, existing.group_key)
        } else {
            existing.group_key
        };
        self.get_group_for_user(actor.user_id, &moved_path)
            .await?
            .context("moved group not found")
    }

    pub async fn delete_group(&self, actor: &NamespaceActor, group_path: &str) -> Result<()> {
        let existing = self
            .get_group_for_user(actor.user_id, group_path)
            .await?
            .context("unknown group")?;
        ensure_role_at_least(existing.current_role, MembershipRole::Owner, "group")?;
        if existing.kind == GroupKind::Personal {
            return Err(anyhow!("personal groups cannot be deleted"));
        }
        sqlx::query_file!("src/sql/db/namespaces/delete_group.sql", group_path)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        let group = self
            .get_group_for_user(actor.user_id, group_path)
            .await?
            .context("unknown group")?;
        if group.visibility == Visibility::Private && group.current_role.is_none() {
            return Err(anyhow!("unknown group"));
        }
        let rows = sqlx::query_file_as!(
            MemberRow,
            "src/sql/db/namespaces/list_group_members.sql",
            group_path
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(member_from_row).collect()
    }

    pub async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        let group = self
            .get_group_for_user(actor.user_id, group_path)
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
        group_path: &str,
        login_name: &str,
    ) -> Result<()> {
        let group = self
            .get_group_for_user(actor.user_id, group_path)
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

    pub async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Vec<GroupRecord>> {
        let rows = sqlx::query_file_as!(
            GroupRow,
            "src/sql/db/namespaces/list_child_groups_for_user.sql",
            user_id,
            group_path
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(group_from_row).collect()
    }

    pub async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope> {
        let Some(user_id) = user_id else {
            return Ok(AccessScope {
                user_id: None,
                include_public: true,
                private_group_ids: Vec::new(),
                group_path,
            });
        };

        let rows = sqlx::query_file_as!(
            PrivateGroupIdRow,
            "src/sql/db/namespaces/resolve_access_scope.sql",
            user_id,
            group_path.as_deref()
        )
        .fetch_all(self.pool())
        .await?;

        Ok(AccessScope {
            user_id: Some(user_id),
            include_public: true,
            private_group_ids: rows.into_iter().map(|row| row.group_id).collect(),
            group_path,
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
        group_path: row.group_path,
        parent_group_path: row.parent_group_path,
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
