use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;

#[derive(Debug, Clone)]
pub struct PersonalAccessTokenRecord {
    pub id: Uuid,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub display_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct NewPersonalAccessToken {
    pub id: Uuid,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub display_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct PersonalAccessTokenRow {
    id: Uuid,
    user_id: i64,
    name: String,
    token_hash: String,
    display_prefix: String,
    scopes: Vec<String>,
    expires_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Database {
    pub async fn insert_personal_access_token(
        &self,
        command: &NewPersonalAccessToken,
    ) -> Result<PersonalAccessTokenRecord> {
        let row = sqlx::query_file_as!(
            PersonalAccessTokenRow,
            "src/sql/db/personal_access_tokens/insert_personal_access_token.sql",
            command.id,
            command.user_id,
            command.name,
            command.token_hash,
            command.display_prefix,
            &command.scopes,
            command.expires_at
        )
        .fetch_one(self.pool())
        .await?;
        Ok(personal_access_token_from_row(row))
    }

    pub async fn list_personal_access_tokens(
        &self,
        user_id: i64,
    ) -> Result<Vec<PersonalAccessTokenRecord>> {
        let rows = sqlx::query_file_as!(
            PersonalAccessTokenRow,
            "src/sql/db/personal_access_tokens/list_personal_access_tokens.sql",
            user_id
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(personal_access_token_from_row)
            .collect())
    }

    pub async fn count_personal_access_tokens(&self, user_id: i64) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/personal_access_tokens/count_personal_access_tokens.sql",
            user_id
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn list_personal_access_tokens_page(
        &self,
        user_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PersonalAccessTokenRecord>> {
        let rows = sqlx::query_file_as!(
            PersonalAccessTokenRow,
            "src/sql/db/personal_access_tokens/list_personal_access_tokens_page.sql",
            user_id,
            limit,
            offset
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(personal_access_token_from_row)
            .collect())
    }

    pub async fn get_personal_access_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessTokenRecord>> {
        let row = sqlx::query_file_as!(
            PersonalAccessTokenRow,
            "src/sql/db/personal_access_tokens/get_personal_access_token_by_hash.sql",
            token_hash
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(personal_access_token_from_row))
    }

    pub async fn revoke_personal_access_token(
        &self,
        token_id: Uuid,
        user_id: i64,
    ) -> Result<Option<PersonalAccessTokenRecord>> {
        let row = sqlx::query_file_as!(
            PersonalAccessTokenRow,
            "src/sql/db/personal_access_tokens/revoke_personal_access_token.sql",
            token_id,
            user_id
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(personal_access_token_from_row))
    }

    pub async fn touch_personal_access_token_last_used(&self, token_id: Uuid) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/personal_access_tokens/touch_personal_access_token_last_used.sql",
            token_id
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

fn personal_access_token_from_row(row: PersonalAccessTokenRow) -> PersonalAccessTokenRecord {
    PersonalAccessTokenRecord {
        id: row.id,
        user_id: row.user_id,
        name: row.name,
        token_hash: row.token_hash,
        display_prefix: row.display_prefix,
        scopes: row.scopes,
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        revoked_at: row.revoked_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
