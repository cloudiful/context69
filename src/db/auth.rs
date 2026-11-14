use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use sqlx::FromRow;

use super::Database;
use super::entity::users;
use crate::domain::UserRecord;

#[derive(Debug, Clone, FromRow)]
struct UserRow {
    id: i64,
    login_name: String,
    display_name: String,
    password_hash: String,
    is_admin: bool,
    disabled_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn auth_database_error(error: sea_orm::DbErr) -> anyhow::Error {
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
    pub async fn get_user_by_login_name(&self, login_name: &str) -> Result<Option<UserRecord>> {
        let row = users::Entity::find()
            .filter(users::Column::LoginName.eq(login_name))
            .one(&self.orm)
            .await
            .map_err(auth_database_error)?;
        Ok(row.map(user_from_model))
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> Result<Option<UserRecord>> {
        let row = users::Entity::find_by_id(user_id)
            .one(&self.orm)
            .await
            .map_err(auth_database_error)?;
        Ok(row.map(user_from_model))
    }

    pub async fn create_user(
        &self,
        login_name: &str,
        display_name: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> Result<UserRecord> {
        let row = users::Entity::insert(users::ActiveModel {
            login_name: Set(login_name.to_owned()),
            display_name: Set(display_name.to_owned()),
            password_hash: Set(password_hash.to_owned()),
            is_admin: Set(is_admin),
            ..Default::default()
        })
        .exec_with_returning(&self.orm)
        .await
        .map_err(auth_database_error)?;
        Ok(user_from_model(row))
    }

    pub async fn count_users(&self, query: &str) -> Result<i64> {
        let trimmed = query.trim_matches(' ');
        let mut selection = users::Entity::find();
        if !trimmed.is_empty() {
            let pattern = format!("%{trimmed}%");
            selection = selection.filter(
                Condition::any()
                    .add(users::Column::LoginName.ilike(pattern.clone()))
                    .add(users::Column::DisplayName.ilike(pattern)),
            );
        }
        Ok(i64::try_from(
            selection
                .count(&self.orm)
                .await
                .map_err(auth_database_error)?,
        )?)
    }

    pub async fn list_users(
        &self,
        query: &str,
        sort_by: Option<&str>,
        sort_direction: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<UserRecord>> {
        let (Ok(limit), Ok(offset)) = (u64::try_from(limit), u64::try_from(offset)) else {
            let rows = sqlx::query_file_as!(
                UserRow,
                "src/sql/db/auth/list_users.sql",
                query,
                sort_by,
                sort_direction,
                limit,
                offset
            )
            .fetch_all(self.pool())
            .await?;
            return Ok(rows.into_iter().map(user_from_row).collect());
        };
        let mut selection = users::Entity::find();
        let trimmed = query.trim_matches(' ');
        if !trimmed.is_empty() {
            let pattern = format!("%{trimmed}%");
            selection = selection.filter(
                Condition::any()
                    .add(users::Column::LoginName.ilike(pattern.clone()))
                    .add(users::Column::DisplayName.ilike(pattern)),
            );
        }
        let column = match sort_by {
            Some("login_name") => Some(users::Column::LoginName),
            Some("display_name") => Some(users::Column::DisplayName),
            Some("created_at") => Some(users::Column::CreatedAt),
            _ => None,
        };
        if let Some(column) = column {
            selection = match sort_direction {
                Some("asc") => selection.order_by_asc(column),
                Some("desc") => selection.order_by_desc(column),
                _ => selection,
            };
        }
        let rows = selection
            .order_by_asc(users::Column::LoginName)
            .order_by_asc(users::Column::Id)
            .limit(limit)
            .offset(offset)
            .all(&self.orm)
            .await
            .map_err(auth_database_error)?;
        Ok(rows.into_iter().map(user_from_model).collect())
    }

    pub async fn update_user(
        &self,
        login_name: &str,
        display_name: Option<&str>,
        is_admin: Option<bool>,
    ) -> Result<Option<UserRecord>> {
        let mut tx = self.pool().begin().await?;
        let existing = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/get_user_for_update_by_login_name.sql",
            login_name
        )
        .fetch_optional(&mut *tx)
        .await?;

        let Some(existing) = existing else {
            tx.rollback().await?;
            return Ok(None);
        };

        let next_display_name = display_name.unwrap_or(existing.display_name.as_str());
        let next_is_admin = is_admin.unwrap_or(existing.is_admin);

        if existing.is_admin && !next_is_admin {
            let admin_count =
                sqlx::query_file_scalar!("src/sql/db/auth/count_active_admin_users.sql")
                    .fetch_one(&mut *tx)
                    .await?;
            if admin_count <= 1 {
                tx.rollback().await?;
                return Err(anyhow::anyhow!("cannot remove the last administrator"));
            }
        }

        let row = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/update_user.sql",
            login_name,
            next_display_name,
            next_is_admin
        )
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(user_from_row(row)))
    }

    pub async fn update_user_password_hash(
        &self,
        login_name: &str,
        password_hash: &str,
    ) -> Result<Option<UserRecord>> {
        let rows = users::Entity::update_many()
            .col_expr(
                users::Column::PasswordHash,
                Expr::value(password_hash.to_owned()),
            )
            .col_expr(users::Column::UpdatedAt, Expr::current_timestamp())
            .filter(users::Column::LoginName.eq(login_name))
            .exec_with_returning(&self.orm)
            .await
            .map_err(auth_database_error)?;
        Ok(rows.into_iter().next().map(user_from_model))
    }

    pub async fn search_user_directory(&self, query: &str, limit: i64) -> Result<Vec<UserRecord>> {
        let normalized_query = format!("%{}%", query.trim().to_lowercase());
        let Ok(limit) = u64::try_from(limit) else {
            let rows = sqlx::query_file_as!(
                UserRow,
                "src/sql/db/auth/search_user_directory.sql",
                normalized_query,
                limit
            )
            .fetch_all(self.pool())
            .await?;
            return Ok(rows.into_iter().map(user_from_row).collect());
        };
        let rows = users::Entity::find()
            .filter(users::Column::DisabledAt.is_null())
            .filter(
                Condition::any()
                    .add(
                        Expr::expr(Func::lower(Expr::col(users::Column::LoginName)))
                            .like(normalized_query.clone()),
                    )
                    .add(
                        Expr::expr(Func::lower(Expr::col(users::Column::DisplayName)))
                            .like(normalized_query),
                    ),
            )
            .order_by_asc(users::Column::LoginName)
            .limit(limit)
            .all(&self.orm)
            .await
            .map_err(auth_database_error)?;
        Ok(rows.into_iter().map(user_from_model).collect())
    }

    pub async fn set_user_disabled_at(
        &self,
        login_name: &str,
        disabled_at: Option<DateTime<Utc>>,
    ) -> Result<Option<UserRecord>> {
        let mut tx = self.pool().begin().await?;
        let existing = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/get_user_for_update_by_login_name.sql",
            login_name
        )
        .fetch_optional(&mut *tx)
        .await?;

        let Some(existing) = existing else {
            tx.rollback().await?;
            return Ok(None);
        };

        if existing.is_admin && disabled_at.is_some() && existing.disabled_at.is_none() {
            let admin_count =
                sqlx::query_file_scalar!("src/sql/db/auth/count_active_admin_users.sql")
                    .fetch_one(&mut *tx)
                    .await?;
            if admin_count <= 1 {
                tx.rollback().await?;
                return Err(anyhow::anyhow!("cannot disable the last administrator"));
            }
        }

        let row = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/set_user_disabled_at.sql",
            login_name,
            disabled_at
        )
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(user_from_row(row)))
    }
}

fn user_from_model(row: users::Model) -> UserRecord {
    UserRecord {
        id: row.id,
        login_name: row.login_name,
        display_name: row.display_name,
        password_hash: row.password_hash,
        is_admin: row.is_admin,
        disabled_at: row.disabled_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn user_from_row(row: UserRow) -> UserRecord {
    UserRecord {
        id: row.id,
        login_name: row.login_name,
        display_name: row.display_name,
        password_hash: row.password_hash,
        is_admin: row.is_admin,
        disabled_at: row.disabled_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
