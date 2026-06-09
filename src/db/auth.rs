use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;
use crate::domain::UserRecord;

#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub id: Uuid,
    pub user_id: i64,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub replaced_by_token_id: Option<Uuid>,
}

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

#[derive(Debug, Clone, FromRow)]
struct RefreshTokenRow {
    id: Uuid,
    user_id: i64,
    token_hash: String,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    replaced_by_token_id: Option<Uuid>,
}

impl Database {
    pub async fn get_user_by_login_name(&self, login_name: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            FROM context69.users
            WHERE login_name = $1
            "#,
        )
        .bind(login_name)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(user_from_row))
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            FROM context69.users
            WHERE id = $1
            "#,
        )
        .bind(user_id)
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
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO context69.users (
                login_name,
                display_name,
                password_hash,
                is_admin
            )
            VALUES ($1, $2, $3, $4)
            RETURNING id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            "#,
        )
        .bind(login_name)
        .bind(display_name)
        .bind(password_hash)
        .bind(is_admin)
        .fetch_one(self.pool())
        .await?;
        Ok(user_from_row(row))
    }

    pub async fn list_users(&self) -> Result<Vec<UserRecord>> {
        let rows = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            FROM context69.users
            ORDER BY login_name
            "#,
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
        let existing = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            FROM context69.users
            WHERE login_name = $1
            FOR UPDATE
            "#,
        )
        .bind(login_name)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(existing) = existing else {
            tx.rollback().await?;
            return Ok(None);
        };

        let next_display_name = display_name.unwrap_or(existing.display_name.as_str());
        let next_is_admin = is_admin.unwrap_or(existing.is_admin);

        if existing.is_admin && !next_is_admin {
            let admin_count = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*)
                FROM context69.users
                WHERE is_admin = true
                  AND disabled_at IS NULL
                "#,
            )
            .fetch_one(&mut *tx)
            .await?;
            if admin_count <= 1 {
                tx.rollback().await?;
                return Err(anyhow::anyhow!("cannot remove the last administrator"));
            }
        }

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            UPDATE context69.users
            SET display_name = $2,
                is_admin = $3,
                updated_at = now()
            WHERE login_name = $1
            RETURNING id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            "#,
        )
        .bind(login_name)
        .bind(next_display_name)
        .bind(next_is_admin)
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
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            UPDATE context69.users
            SET password_hash = $2,
                updated_at = now()
            WHERE login_name = $1
            RETURNING id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            "#,
        )
        .bind(login_name)
        .bind(password_hash)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(user_from_row))
    }

    pub async fn search_user_directory(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<UserRecord>> {
        let normalized_query = format!("%{}%", query.trim().to_lowercase());
        let rows = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            FROM context69.users
            WHERE disabled_at IS NULL
              AND (
                lower(login_name) LIKE $1
                OR lower(display_name) LIKE $1
              )
            ORDER BY login_name
            LIMIT $2
            "#,
        )
        .bind(normalized_query)
        .bind(limit)
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
        let existing = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            FROM context69.users
            WHERE login_name = $1
            FOR UPDATE
            "#,
        )
        .bind(login_name)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(existing) = existing else {
            tx.rollback().await?;
            return Ok(None);
        };

        if existing.is_admin && disabled_at.is_some() && existing.disabled_at.is_none() {
            let admin_count = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*)
                FROM context69.users
                WHERE is_admin = true
                  AND disabled_at IS NULL
                "#,
            )
            .fetch_one(&mut *tx)
            .await?;
            if admin_count <= 1 {
                tx.rollback().await?;
                return Err(anyhow::anyhow!("cannot disable the last administrator"));
            }
        }

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            UPDATE context69.users
            SET disabled_at = $2,
                updated_at = now()
            WHERE login_name = $1
            RETURNING id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
            "#,
        )
        .bind(login_name)
        .bind(disabled_at)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(user_from_row(row)))
    }

    pub async fn insert_refresh_token(
        &self,
        id: Uuid,
        user_id: i64,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<RefreshTokenRecord> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            r#"
            INSERT INTO context69.refresh_tokens (
                id,
                user_id,
                token_hash,
                expires_at
            )
            VALUES ($1, $2, $3, $4)
            RETURNING id, user_id, token_hash, expires_at, revoked_at, replaced_by_token_id
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .fetch_one(self.pool())
        .await?;
        Ok(refresh_token_from_row(row))
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRecord>> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            r#"
            SELECT id, user_id, token_hash, expires_at, revoked_at, replaced_by_token_id
            FROM context69.refresh_tokens
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(refresh_token_from_row))
    }

    pub async fn rotate_refresh_token(
        &self,
        current_id: Uuid,
        current_hash: &str,
        replacement_id: Uuid,
        replacement_hash: &str,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> Result<RefreshTokenRecord> {
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            r#"
            UPDATE context69.refresh_tokens
            SET revoked_at = now(),
                replaced_by_token_id = $2,
                last_used_at = now()
            WHERE id = $1 AND token_hash = $3
            "#,
        )
        .bind(current_id)
        .bind(replacement_id)
        .bind(current_hash)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            r#"
            INSERT INTO context69.refresh_tokens (
                id,
                user_id,
                token_hash,
                expires_at
            )
            VALUES ($1, $2, $3, $4)
            RETURNING id, user_id, token_hash, expires_at, revoked_at, replaced_by_token_id
            "#,
        )
        .bind(replacement_id)
        .bind(user_id)
        .bind(replacement_hash)
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(refresh_token_from_row(row))
    }

    pub async fn revoke_refresh_token_by_hash(&self, token_hash: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE context69.refresh_tokens
            SET revoked_at = COALESCE(revoked_at, now()),
                last_used_at = now()
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .execute(self.pool())
        .await?;
        Ok(())
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

fn refresh_token_from_row(row: RefreshTokenRow) -> RefreshTokenRecord {
    RefreshTokenRecord {
        id: row.id,
        user_id: row.user_id,
        token_hash: row.token_hash,
        expires_at: row.expires_at,
        revoked_at: row.revoked_at,
        replaced_by_token_id: row.replaced_by_token_id,
    }
}
