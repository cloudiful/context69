use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use super::Database;
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

impl Database {
    pub async fn get_user_by_login_name(&self, login_name: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/get_user_by_login_name.sql",
            login_name
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(user_from_row))
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> Result<Option<UserRecord>> {
        let row = sqlx::query_file_as!(UserRow, "src/sql/db/auth/get_user_by_id.sql", user_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(user_from_row))
    }

    pub async fn create_user(
        &self,
        login_name: &str,
        display_name: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> Result<UserRecord> {
        let row = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/create_user.sql",
            login_name,
            display_name,
            password_hash,
            is_admin
        )
        .fetch_one(self.pool())
        .await?;
        Ok(user_from_row(row))
    }

    pub async fn count_users(&self, query: &str) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/auth/count_users.sql", query)
                .fetch_one(self.pool())
                .await?,
        )
    }

    pub async fn list_users(
        &self,
        query: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<UserRecord>> {
        let rows = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/list_users.sql",
            query,
            limit,
            offset
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(user_from_row).collect())
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
        let row = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/update_user_password_hash.sql",
            login_name,
            password_hash
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(user_from_row))
    }

    pub async fn search_user_directory(&self, query: &str, limit: i64) -> Result<Vec<UserRecord>> {
        let normalized_query = format!("%{}%", query.trim().to_lowercase());
        let rows = sqlx::query_file_as!(
            UserRow,
            "src/sql/db/auth/search_user_directory.sql",
            normalized_query,
            limit
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(user_from_row).collect())
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
