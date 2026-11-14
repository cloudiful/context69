use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, Value};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use uuid::Uuid;

use super::Database;
use super::entity::personal_access_tokens;

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

impl Database {
    pub async fn insert_personal_access_token(
        &self,
        command: &NewPersonalAccessToken,
    ) -> Result<PersonalAccessTokenRecord> {
        let row = personal_access_tokens::Entity::insert(personal_access_tokens::ActiveModel {
            id: Set(command.id),
            user_id: Set(command.user_id),
            name: Set(command.name.clone()),
            token_hash: Set(command.token_hash.clone()),
            display_prefix: Set(command.display_prefix.clone()),
            scopes: Set(command.scopes.clone()),
            expires_at: Set(command.expires_at),
            ..Default::default()
        })
        .exec_with_returning(&self.orm)
        .await?;
        Ok(personal_access_token_from_model(row))
    }

    pub async fn list_personal_access_tokens(
        &self,
        user_id: i64,
    ) -> Result<Vec<PersonalAccessTokenRecord>> {
        let rows = personal_access_tokens::Entity::find()
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .order_by_desc(personal_access_tokens::Column::CreatedAt)
            .order_by_desc(personal_access_tokens::Column::Id)
            .all(&self.orm)
            .await?;
        Ok(rows
            .into_iter()
            .map(personal_access_token_from_model)
            .collect())
    }

    pub async fn count_personal_access_tokens(&self, user_id: i64) -> Result<i64> {
        let count = personal_access_tokens::Entity::find()
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .count(&self.orm)
            .await?;
        i64::try_from(count).context("personal access token count out of range")
    }

    pub async fn list_personal_access_tokens_page(
        &self,
        user_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PersonalAccessTokenRecord>> {
        let limit = u64::try_from(limit)
            .context("personal access token page limit must be non-negative")?;
        let offset = u64::try_from(offset)
            .context("personal access token page offset must be non-negative")?;
        let rows = personal_access_tokens::Entity::find()
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .order_by_desc(personal_access_tokens::Column::CreatedAt)
            .order_by_desc(personal_access_tokens::Column::Id)
            .limit(limit)
            .offset(offset)
            .all(&self.orm)
            .await?;
        Ok(rows
            .into_iter()
            .map(personal_access_token_from_model)
            .collect())
    }

    pub async fn get_personal_access_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessTokenRecord>> {
        let row = personal_access_tokens::Entity::find()
            .filter(personal_access_tokens::Column::TokenHash.eq(token_hash))
            .one(&self.orm)
            .await?;
        Ok(row.map(personal_access_token_from_model))
    }

    pub async fn revoke_personal_access_token(
        &self,
        token_id: Uuid,
        user_id: i64,
    ) -> Result<Option<PersonalAccessTokenRecord>> {
        let rows = personal_access_tokens::Entity::update_many()
            .col_expr(
                personal_access_tokens::Column::RevokedAt,
                Expr::cust("COALESCE(revoked_at, now())"),
            )
            .col_expr(
                personal_access_tokens::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .filter(personal_access_tokens::Column::Id.eq(token_id))
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .exec_with_returning(&self.orm)
            .await?;
        Ok(rows
            .into_iter()
            .next()
            .map(personal_access_token_from_model))
    }

    /// Record approximate usage for a batch of tokens.
    ///
    /// The statement is guarded, so it writes only the tokens whose usage
    /// timestamp is missing or stale, and only while the token is still active.
    /// `updated_at` stays untouched: usage is telemetry, not a change to the
    /// token's configuration.
    pub async fn touch_personal_access_tokens_last_used(&self, token_ids: &[Uuid]) -> Result<u64> {
        if token_ids.is_empty() {
            return Ok(0);
        }
        let result = personal_access_tokens::Entity::update_many()
            .col_expr(
                personal_access_tokens::Column::LastUsedAt,
                Expr::cust("statement_timestamp()"),
            )
            .filter(Expr::cust_with_values(
                "id = ANY($1::uuid[]) \
                 AND revoked_at IS NULL \
                 AND (expires_at IS NULL OR expires_at > statement_timestamp()) \
                 AND (last_used_at IS NULL OR last_used_at < date_trunc('day', \
                 statement_timestamp() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC')",
                [Value::from(token_ids.to_vec())],
            ))
            .exec(&self.orm)
            .await?;
        Ok(result.rows_affected)
    }
}

fn personal_access_token_from_model(
    row: personal_access_tokens::Model,
) -> PersonalAccessTokenRecord {
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
