use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use context69_namespace::{
    AccessScope, CreateGroupInput, GroupRecord, MoveGroupInput, NamespaceActor,
    NamespaceMemberRecord, Page, PageRequest, PersonalGroupRecord, UpdateGroupInput,
    UpsertMembershipInput,
};
use sea_orm::sea_query::{
    Alias, Asterisk, Condition, Expr, ExprTrait, Func, InsertStatement, JoinType, OnConflict,
    Order, Query, SelectStatement, extension::postgres::PgExpr,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, TransactionTrait,
};
use sqlx::FromRow;

use super::Database;
use super::entity::{group_memberships, groups, users};
use crate::{
    contracts::{GroupKind, MembershipRole, Pagination, Visibility},
    domain::UserRecord,
    domain_errors::DomainError,
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

fn member_query(group_path: &str, query: &str) -> SelectStatement {
    let gm = Alias::new("gm");
    let u = Alias::new("u");
    let g = Alias::new("g");
    let mut select = Query::select();
    select
        .from_as(
            (Alias::new("context69"), group_memberships::Entity),
            gm.clone(),
        )
        .join_as(
            JoinType::InnerJoin,
            (Alias::new("context69"), users::Entity),
            u.clone(),
            Expr::col((u.clone(), users::Column::Id))
                .equals((gm.clone(), group_memberships::Column::UserId)),
        )
        .join_as(
            JoinType::InnerJoin,
            (Alias::new("context69"), groups::Entity),
            g.clone(),
            Expr::col((g.clone(), groups::Column::Id))
                .equals((gm, group_memberships::Column::GroupId)),
        )
        .and_where(Expr::col((g, groups::Column::FullPath)).eq(group_path));
    let trimmed = query.trim_matches(' ');
    if !trimmed.is_empty() {
        let pattern = format!("%{trimmed}%");
        select.cond_where(
            Condition::any()
                .add(Expr::col((u.clone(), users::Column::LoginName)).ilike(pattern.clone()))
                .add(Expr::col((u, users::Column::DisplayName)).ilike(pattern)),
        );
    }
    select.to_owned()
}

fn group_insert_query(
    parent_id: Option<i64>,
    group_key: &str,
    name: &str,
    visibility: Visibility,
    kind: GroupKind,
    user_id: i64,
) -> Result<InsertStatement> {
    let path = if let Some(parent_id) = parent_id {
        Expr::from(
            Query::select()
                .expr(
                    Expr::col(groups::Column::FullPath)
                        .concatenate(Expr::value(format!("/{group_key}"))),
                )
                .from((Alias::new("context69"), groups::Entity))
                .and_where(Expr::col(groups::Column::Id).eq(parent_id))
                .to_owned(),
        )
    } else {
        Expr::value(group_key.to_owned())
    };
    let select = Query::select()
        .expr(Expr::value(parent_id))
        .expr(Expr::value(group_key.to_owned()))
        .expr(path)
        .expr(Expr::value(name.to_owned()))
        .expr(Expr::value(visibility.as_str().to_owned()))
        .expr(Expr::value(kind.as_str().to_owned()))
        .expr(Expr::value(user_id))
        .expr(Expr::value(user_id))
        .to_owned();
    Ok(Query::insert()
        .into_table((Alias::new("context69"), groups::Entity))
        .columns([
            groups::Column::ParentGroupId,
            groups::Column::GroupKey,
            groups::Column::FullPath,
            groups::Column::Name,
            groups::Column::Visibility,
            groups::Column::Kind,
            groups::Column::OwnerUserId,
            groups::Column::CreatedByUserId,
        ])
        .select_from(select)?
        .returning_col(groups::Column::Id)
        .to_owned())
}

fn namespace_database_error(error: sea_orm::DbErr) -> anyhow::Error {
    use sea_orm::{DbErr, RuntimeErr};
    let (error, wrap): (_, fn(RuntimeErr) -> DbErr) = match error {
        DbErr::Exec(RuntimeErr::SqlxError(error)) => (error, DbErr::Exec),
        DbErr::Query(RuntimeErr::SqlxError(error)) => (error, DbErr::Query),
        DbErr::Conn(RuntimeErr::SqlxError(error)) => (error, DbErr::Conn),
        error => return error.into(),
    };
    match std::sync::Arc::try_unwrap(error) {
        Ok(error) => error.into(),
        Err(error) => wrap(RuntimeErr::SqlxError(error)).into(),
    }
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
        let tx = self.orm.begin().await.map_err(namespace_database_error)?;
        let group = groups::Entity::insert(groups::ActiveModel {
            parent_group_id: Set(None),
            group_key: Set(group_key.clone()),
            full_path: Set(group_key.clone()),
            name: Set(user.display_name.clone()),
            visibility: Set("private".to_owned()),
            kind: Set("personal".to_owned()),
            owner_user_id: Set(Some(user.id)),
            created_by_user_id: Set(Some(user.id)),
            ..Default::default()
        })
        .exec_with_returning(&tx)
        .await
        .map_err(namespace_database_error)?;
        let group_id = group.id;
        group_memberships::Entity::insert(group_memberships::ActiveModel {
            group_id: Set(group_id),
            user_id: Set(user.id),
            role: Set("owner".to_owned()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::columns([
                group_memberships::Column::GroupId,
                group_memberships::Column::UserId,
            ])
            .update_column(group_memberships::Column::Role)
            .value(
                group_memberships::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .to_owned(),
        )
        .exec(&tx)
        .await
        .map_err(namespace_database_error)?;
        tx.commit().await.map_err(namespace_database_error)?;

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
        let g = Alias::new("g");
        let gm = Alias::new("gm");
        let statement = Query::select()
            .expr_as(
                Expr::col((g.clone(), groups::Column::Id)),
                Alias::new("group_id"),
            )
            .expr_as(
                Expr::col((g.clone(), groups::Column::FullPath)),
                Alias::new("group_path"),
            )
            .column((gm.clone(), group_memberships::Column::Role))
            .from_as((Alias::new("context69"), groups::Entity), g.clone())
            .join_as(
                JoinType::InnerJoin,
                (Alias::new("context69"), group_memberships::Entity),
                gm.clone(),
                Expr::col((gm.clone(), group_memberships::Column::GroupId))
                    .equals((g.clone(), groups::Column::Id))
                    .and(Expr::col((gm, group_memberships::Column::UserId)).eq(user_id)),
            )
            .and_where(Expr::col((g.clone(), groups::Column::Kind)).eq("personal"))
            .and_where(Expr::col((g.clone(), groups::Column::OwnerUserId)).eq(user_id))
            .order_by((g, groups::Column::Id), Order::Asc)
            .limit(1)
            .to_owned();
        self.orm
            .query_one(&statement)
            .await
            .map_err(namespace_database_error)?
            .map(|row| {
                Ok(PersonalGroupRecord {
                    group_id: row.try_get("", "group_id")?,
                    group_path: row.try_get("", "group_path")?,
                    role: row
                        .try_get::<String>("", "role")?
                        .parse()
                        .unwrap_or(MembershipRole::Owner),
                })
            })
            .transpose()
    }

    pub async fn list_groups_for_user(
        &self,
        user_id: i64,
        request: &PageRequest,
    ) -> Result<Page<GroupRecord>> {
        let total = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/count_groups_for_user.sql",
            user_id,
            &request.query,
            request.visibility.map(Visibility::as_str),
            request.kind.map(GroupKind::as_str)
        )
        .fetch_one(self.pool())
        .await?;
        let rows = sqlx::query_file_as!(
            GroupRow,
            "src/sql/db/namespaces/list_groups_for_user.sql",
            user_id,
            &request.query,
            i64::from(request.page_size),
            page_offset(request)?,
            request.sort.map(|sort| sort.column),
            request.sort.map(|sort| sort.direction.as_str()),
            request.visibility.map(Visibility::as_str),
            request.kind.map(GroupKind::as_str)
        )
        .fetch_all(self.pool())
        .await?;
        page_result(
            rows.into_iter()
                .map(group_from_row)
                .collect::<Result<_>>()?,
            total,
            request,
        )
    }

    pub async fn search_groups_for_user(
        &self,
        user_id: i64,
        query: &str,
        limit: u32,
    ) -> Result<Vec<GroupRecord>> {
        let limit = i64::from(limit.clamp(1, 20));
        let rows = sqlx::query_file_as!(
            GroupRow,
            "src/sql/db/namespaces/search_groups_for_user.sql",
            user_id,
            query,
            limit
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

    pub async fn get_group_by_id(&self, group_id: i64) -> Result<Option<GroupRecord>> {
        let g = Alias::new("g");
        let parent = Alias::new("parent");
        let statement = Query::select()
            .columns([
                (g.clone(), groups::Column::Id),
                (g.clone(), groups::Column::ParentGroupId),
                (g.clone(), groups::Column::GroupKey),
                (g.clone(), groups::Column::Name),
                (g.clone(), groups::Column::Visibility),
                (g.clone(), groups::Column::Kind),
                (g.clone(), groups::Column::OwnerUserId),
                (g.clone(), groups::Column::CreatedAt),
                (g.clone(), groups::Column::UpdatedAt),
            ])
            .expr_as(
                Expr::col((g.clone(), groups::Column::FullPath)),
                Alias::new("group_path"),
            )
            .expr_as(
                Expr::col((parent.clone(), groups::Column::FullPath)),
                Alias::new("parent_group_path"),
            )
            .from_as((Alias::new("context69"), groups::Entity), g.clone())
            .join_as(
                JoinType::LeftJoin,
                (Alias::new("context69"), groups::Entity),
                parent.clone(),
                Expr::col((parent, groups::Column::Id))
                    .equals((g.clone(), groups::Column::ParentGroupId)),
            )
            .and_where(Expr::col((g, groups::Column::Id)).eq(group_id))
            .to_owned();
        self.orm
            .query_one(&statement)
            .await
            .map_err(namespace_database_error)?
            .map(|row| {
                group_from_row(GroupRow {
                    id: row.try_get("", "id")?,
                    parent_group_id: row.try_get("", "parent_group_id")?,
                    group_path: row.try_get("", "group_path")?,
                    parent_group_path: row.try_get("", "parent_group_path")?,
                    group_key: row.try_get("", "group_key")?,
                    name: row.try_get("", "name")?,
                    visibility: row.try_get("", "visibility")?,
                    kind: row.try_get("", "kind")?,
                    owner_user_id: row.try_get("", "owner_user_id")?,
                    created_at: row.try_get("", "created_at")?,
                    updated_at: row.try_get("", "updated_at")?,
                    current_role_rank: None,
                })
            })
            .transpose()
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

        let tx = self.orm.begin().await.map_err(namespace_database_error)?;
        let parent_id = parent_group.as_ref().map(|group| group.id);
        let statement = group_insert_query(
            parent_id,
            group_key,
            name,
            request.visibility,
            kind,
            actor.user_id,
        )?;
        let group_id = tx
            .query_one(&statement)
            .await
            .map_err(namespace_database_error)?
            .ok_or(sqlx::Error::RowNotFound)?
            .try_get::<i64>("", "id")?;
        group_memberships::Entity::insert(group_memberships::ActiveModel {
            group_id: Set(group_id),
            user_id: Set(actor.user_id),
            role: Set("owner".to_owned()),
            ..Default::default()
        })
        .exec(&tx)
        .await
        .map_err(namespace_database_error)?;
        tx.commit().await.map_err(namespace_database_error)?;

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

        groups::Entity::update_many()
            .col_expr(groups::Column::Name, Expr::value(next_name.to_owned()))
            .col_expr(
                groups::Column::Visibility,
                Expr::value(next_visibility.as_str().to_owned()),
            )
            .col_expr(groups::Column::UpdatedAt, Expr::current_timestamp())
            .filter(groups::Column::FullPath.eq(group_path))
            .exec(&self.orm)
            .await
            .map_err(namespace_database_error)?;

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

        let target_parent = if let Some(target_parent_path) =
            request.target_parent_group_path.as_deref()
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
            if target.visibility == Visibility::Private && existing.visibility == Visibility::Public
            {
                return Err(anyhow!(
                    "group visibility cannot be broader than parent group visibility"
                ));
            }
            Some(target)
        } else {
            None
        };

        let target_parent_id = target_parent.as_ref().map(|group| group.id);
        // The exclusive ownership locks cover the moved subtree and both the
        // old and new ancestor chains, so a move cannot interleave with a
        // registration or deletion that would cascade it. The re-parent, path
        // rebuild, and locks commit together.
        let mut tx = self.pool().begin().await?;
        sqlx::query_file!(
            "src/sql/db/namespaces/lock_group_move_ownership.sql",
            existing.id,
            target_parent_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/db/namespaces/move_group_update_parent.sql",
            existing.id,
            target_parent_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!("src/sql/db/namespaces/rebuild_group_paths.sql")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

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
        // The exclusive ownership lock, the row lock, the Git-ownership check,
        // and the delete are one transaction. The lock is taken before the row
        // lock so a concurrent source registration holding the shared ancestor
        // lock can finish its insert and be counted instead of deadlocking.
        let mut tx = self.pool().begin().await?;
        sqlx::query_file!(
            "src/sql/db/namespaces/lock_group_ownership_for_deletion.sql",
            existing.id
        )
        .execute(&mut *tx)
        .await?;
        let locked = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/lock_group_for_deletion.sql",
            existing.id
        )
        .fetch_optional(&mut *tx)
        .await?;
        if locked.is_none() {
            tx.rollback().await?;
            return Err(anyhow!("unknown group"));
        }
        let owned = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/count_group_git_owned_state.sql",
            existing.id
        )
        .fetch_one(&mut *tx)
        .await?;
        if owned.unwrap_or(0) > 0 {
            tx.rollback().await?;
            return Err(anyhow!(DomainError::conflict(
                "git_group_owned_state_present"
            )));
        }
        sqlx::query_file!("src/sql/db/namespaces/delete_group.sql", existing.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &PageRequest,
    ) -> Result<Page<NamespaceMemberRecord>> {
        let group = self
            .get_group_for_user(actor.user_id, group_path)
            .await?
            .context("unknown group")?;
        if group.visibility == Visibility::Private && group.current_role.is_none() {
            return Err(anyhow!("unknown group"));
        }
        let mut count = member_query(group_path, &request.query);
        count.expr_as(Func::count(Expr::col(Asterisk)), Alias::new("count"));
        let total: i64 = self
            .orm
            .query_one(&count)
            .await
            .map_err(namespace_database_error)?
            .ok_or(sqlx::Error::RowNotFound)?
            .try_get("", "count")?;
        let mut select = member_query(group_path, &request.query);
        let u = Alias::new("u");
        let gm = Alias::new("gm");
        select
            .expr_as(
                Expr::col((u.clone(), users::Column::Id)),
                Alias::new("user_id"),
            )
            .columns([
                (u.clone(), users::Column::LoginName),
                (u.clone(), users::Column::DisplayName),
            ])
            .column((gm.clone(), group_memberships::Column::Role));
        if let Some(sort) = request.sort {
            let order = match sort.direction.as_str() {
                "asc" => Some(Order::Asc),
                "desc" => Some(Order::Desc),
                _ => None,
            };
            if let Some(order) = order {
                match sort.column {
                    "login_name" => {
                        select.order_by((u.clone(), users::Column::LoginName), order);
                    }
                    "display_name" => {
                        select.order_by((u.clone(), users::Column::DisplayName), order);
                    }
                    "role" => {
                        let role = Expr::col((gm, group_memberships::Column::Role));
                        select.order_by_expr(
                            Expr::case(role.clone().eq("owner"), 3)
                                .case(role.eq("maintainer"), 2)
                                .finally(1)
                                .into(),
                            order,
                        );
                    }
                    _ => {}
                }
            }
        }
        select
            .order_by((u.clone(), users::Column::LoginName), Order::Asc)
            .order_by((u, users::Column::Id), Order::Asc)
            .limit(u64::from(request.page_size))
            .offset(u64::try_from(page_offset(request)?)?);
        let rows = self
            .orm
            .query_all(&select)
            .await
            .map_err(namespace_database_error)?
            .into_iter()
            .map(|row| {
                Ok(MemberRow {
                    user_id: row.try_get("", "user_id")?,
                    login_name: row.try_get("", "login_name")?,
                    display_name: row.try_get("", "display_name")?,
                    role: row.try_get("", "role")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        page_result(
            rows.into_iter()
                .map(member_from_row)
                .collect::<Result<_>>()?,
            total,
            request,
        )
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
        group_memberships::Entity::insert(group_memberships::ActiveModel {
            group_id: Set(group.id),
            user_id: Set(user.id),
            role: Set(request.role.as_str().to_owned()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::columns([
                group_memberships::Column::GroupId,
                group_memberships::Column::UserId,
            ])
            .update_column(group_memberships::Column::Role)
            .value(
                group_memberships::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .to_owned(),
        )
        .exec(&self.orm)
        .await
        .map_err(namespace_database_error)?;
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
        group_memberships::Entity::delete_by_id((group.id, user.id))
            .exec(&self.orm)
            .await
            .map_err(namespace_database_error)?;
        Ok(())
    }

    pub async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
        request: &PageRequest,
    ) -> Result<Page<GroupRecord>> {
        let total = sqlx::query_file_scalar!(
            "src/sql/db/namespaces/count_child_groups_for_user.sql",
            user_id,
            group_path,
            &request.query,
            request.visibility.map(Visibility::as_str),
            request.kind.map(GroupKind::as_str)
        )
        .fetch_one(self.pool())
        .await?;
        let rows = sqlx::query_file_as!(
            GroupRow,
            "src/sql/db/namespaces/list_child_groups_for_user.sql",
            user_id,
            group_path,
            &request.query,
            i64::from(request.page_size),
            page_offset(request)?,
            request.sort.map(|sort| sort.column),
            request.sort.map(|sort| sort.direction.as_str()),
            request.visibility.map(Visibility::as_str),
            request.kind.map(GroupKind::as_str)
        )
        .fetch_all(self.pool())
        .await?;
        page_result(
            rows.into_iter()
                .map(group_from_row)
                .collect::<Result<_>>()?,
            total,
            request,
        )
    }

    pub async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope> {
        let scoped_group_id = self.resolve_group_id_by_path(group_path.as_deref()).await?;
        let Some(user_id) = user_id else {
            return Ok(AccessScope {
                user_id: None,
                include_public: true,
                private_group_ids: Vec::new(),
                group_path,
                scoped_group_id,
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
            scoped_group_id,
        })
    }

    async fn resolve_group_id_by_path(&self, group_path: Option<&str>) -> Result<Option<i64>> {
        let Some(group_path) = group_path.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        Ok(groups::Entity::find()
            .filter(groups::Column::FullPath.eq(group_path))
            .one(&self.orm)
            .await
            .map_err(namespace_database_error)?
            .map(|row| row.id))
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

fn page_offset(request: &PageRequest) -> Result<i64> {
    Pagination::offset(request.page, request.page_size)
}

fn page_result<T>(items: Vec<T>, total: i64, request: &PageRequest) -> Result<Page<T>> {
    let total = u64::try_from(total).context("negative page count")?;
    Ok(Page {
        items,
        pagination: Pagination::try_new(request.page, request.page_size, total)?,
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

#[cfg(test)]
mod tests {
    use super::{GroupKind, Visibility, group_insert_query, member_query};
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn child_group_path_is_resolved_in_insert_statement() {
        let sql = group_insert_query(
            Some(7),
            "child",
            "Child",
            Visibility::Private,
            GroupKind::Shared,
            4,
        )
        .unwrap()
        .to_string(PostgresQueryBuilder);
        assert!(
            sql.contains("INSERT INTO \"context69\".\"groups\""),
            "{sql}"
        );
        assert!(
            sql.contains(
                "SELECT \"full_path\" || '/child' FROM \"context69\".\"groups\" WHERE \"id\" = 7"
            ),
            "{sql}"
        );
        assert!(sql.contains("RETURNING \"id\""), "{sql}");
    }

    #[test]
    fn member_query_keeps_group_scope_and_search() {
        let sql = member_query("team", " a ").to_string(PostgresQueryBuilder);
        assert!(sql.contains("\"g\".\"full_path\" = 'team'"), "{sql}");
        assert!(sql.contains("\"u\".\"login_name\" ILIKE '%a%'"), "{sql}");
        assert!(sql.contains("\"u\".\"display_name\" ILIKE '%a%'"), "{sql}");
    }
}
