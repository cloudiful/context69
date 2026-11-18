use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
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
    pub group_scope_all: bool,
    pub group_ids: Vec<i64>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_used_precision: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub rotated_from_token_id: Option<Uuid>,
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
    pub group_scope_all: bool,
    pub group_ids: Vec<i64>,
    pub expires_at: DateTime<Utc>,
    pub rotated_from_token_id: Option<Uuid>,
}

/// The configuration a caller read before proposing an edit or rotation.
///
/// Telemetry, creation time, and the stored secret are intentionally absent:
/// a concurrent use observation must not turn an otherwise current edit into a
/// conflict, while any configuration or revocation change must.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalAccessTokenSnapshot {
    name: String,
    scopes: Vec<String>,
    group_scope_all: bool,
    group_ids: Vec<i64>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl PersonalAccessTokenSnapshot {
    pub fn from_record(record: &PersonalAccessTokenRecord) -> Self {
        Self {
            name: record.name.clone(),
            scopes: record.scopes.clone(),
            group_scope_all: record.group_scope_all,
            group_ids: record.group_ids.clone(),
            expires_at: record.expires_at,
            revoked_at: record.revoked_at,
        }
    }

    pub fn matches(&self, record: &PersonalAccessTokenRecord) -> bool {
        self == &Self::from_record(record)
    }
}

/// The new secret material for a rotation; inherited configuration comes from
/// the locked old row, never from the caller's stale read.
pub struct NewPersonalAccessTokenSecret {
    pub id: Uuid,
    pub user_id: i64,
    pub token_hash: String,
    pub display_prefix: String,
    pub expires_at: DateTime<Utc>,
    pub rotated_from_token_id: Uuid,
}

/// The editable configuration submitted with a locked update.
pub struct PersonalAccessTokenUpdate {
    pub name: String,
    pub scopes: Vec<String>,
    pub group_scope_all: bool,
    pub group_ids: Vec<i64>,
}

/// The result of a locked token mutation.
#[derive(Debug)]
pub enum PersonalAccessTokenMutation {
    Applied(Box<PersonalAccessTokenRecord>),
    Stale,
    NotFound,
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
            group_scope_all: Set(command.group_scope_all),
            group_ids: Set(command.group_ids.clone()),
            expires_at: Set(command.expires_at),
            rotated_from_token_id: Set(command.rotated_from_token_id),
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

    pub async fn get_personal_access_token(
        &self,
        token_id: Uuid,
        user_id: i64,
    ) -> Result<Option<PersonalAccessTokenRecord>> {
        let row = personal_access_tokens::Entity::find()
            .filter(personal_access_tokens::Column::Id.eq(token_id))
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .one(&self.orm)
            .await?;
        Ok(row.map(personal_access_token_from_model))
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

    /// Apply a caller-merged configuration update under the row lock.
    ///
    /// The caller's snapshot must still match the locked row, otherwise the
    /// update is rejected as stale instead of merging against permissions that
    /// another edit already changed. Only the editable fields change: expiry,
    /// revocation, and usage stay untouched, so editing an expired or revoked
    /// token never revives it.
    pub async fn update_personal_access_token(
        &self,
        token_id: Uuid,
        user_id: i64,
        expected: &PersonalAccessTokenSnapshot,
        update: &PersonalAccessTokenUpdate,
    ) -> Result<PersonalAccessTokenMutation> {
        let transaction = self.orm.begin().await?;
        let Some(current) = personal_access_tokens::Entity::find()
            .filter(personal_access_tokens::Column::Id.eq(token_id))
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .lock_exclusive()
            .one(&transaction)
            .await?
        else {
            transaction.rollback().await?;
            return Ok(PersonalAccessTokenMutation::NotFound);
        };
        let current = personal_access_token_from_model(current);
        if !expected.matches(&current) {
            transaction.rollback().await?;
            return Ok(PersonalAccessTokenMutation::Stale);
        }
        let rows = personal_access_tokens::Entity::update_many()
            .col_expr(
                personal_access_tokens::Column::Name,
                Expr::value(update.name.clone()),
            )
            .col_expr(
                personal_access_tokens::Column::Scopes,
                Expr::value(update.scopes.clone()),
            )
            .col_expr(
                personal_access_tokens::Column::GroupScopeAll,
                Expr::value(update.group_scope_all),
            )
            .col_expr(
                personal_access_tokens::Column::GroupIds,
                Expr::value(update.group_ids.clone()),
            )
            .col_expr(
                personal_access_tokens::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .filter(personal_access_tokens::Column::Id.eq(token_id))
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .exec_with_returning(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(match rows.into_iter().next() {
            Some(row) => PersonalAccessTokenMutation::Applied(Box::new(
                personal_access_token_from_model(row),
            )),
            None => PersonalAccessTokenMutation::NotFound,
        })
    }

    /// Replace a token with a freshly generated one in one transaction.
    ///
    /// The old row is locked and compared with the caller's snapshot before any
    /// mutation. The replacement inherits the locked row's configuration, not a
    /// stale read, so a concurrent edit can never be bypassed by issuing a token
    /// with wider permissions. A failure at any step rolls back, so a valid old
    /// token is never destroyed before its replacement exists.
    pub async fn rotate_personal_access_token(
        &self,
        old_token_id: Uuid,
        user_id: i64,
        expected: &PersonalAccessTokenSnapshot,
        new_token: &NewPersonalAccessTokenSecret,
    ) -> Result<PersonalAccessTokenMutation> {
        let transaction = self.orm.begin().await?;
        let Some(old) = personal_access_tokens::Entity::find()
            .filter(personal_access_tokens::Column::Id.eq(old_token_id))
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .lock_exclusive()
            .one(&transaction)
            .await?
        else {
            transaction.rollback().await?;
            return Ok(PersonalAccessTokenMutation::NotFound);
        };
        let old = personal_access_token_from_model(old);
        if !expected.matches(&old) {
            transaction.rollback().await?;
            return Ok(PersonalAccessTokenMutation::Stale);
        }
        let replacement =
            personal_access_tokens::Entity::insert(personal_access_tokens::ActiveModel {
                id: Set(new_token.id),
                user_id: Set(new_token.user_id),
                name: Set(old.name.clone()),
                token_hash: Set(new_token.token_hash.clone()),
                display_prefix: Set(new_token.display_prefix.clone()),
                scopes: Set(old.scopes.clone()),
                group_scope_all: Set(old.group_scope_all),
                group_ids: Set(old.group_ids.clone()),
                expires_at: Set(new_token.expires_at),
                rotated_from_token_id: Set(Some(new_token.rotated_from_token_id)),
                ..Default::default()
            })
            .exec_with_returning(&transaction)
            .await?;
        personal_access_tokens::Entity::update_many()
            .col_expr(
                personal_access_tokens::Column::RevokedAt,
                Expr::cust("COALESCE(revoked_at, now())"),
            )
            .col_expr(
                personal_access_tokens::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .filter(personal_access_tokens::Column::Id.eq(old.id))
            .filter(personal_access_tokens::Column::UserId.eq(user_id))
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(PersonalAccessTokenMutation::Applied(Box::new(
            personal_access_token_from_model(replacement),
        )))
    }

    /// Record successful authorized uses for a batch of tokens.
    ///
    /// Each observation keeps the instant the request was authorized, not the
    /// later flush instant, and the monotonic guard only moves the stored value
    /// forward so an out-of-order asynchronous write cannot roll it back.
    /// `updated_at` stays untouched: usage is telemetry, not a configuration
    /// change.
    pub async fn touch_personal_access_tokens_last_used(
        &self,
        observations: &[(Uuid, DateTime<Utc>)],
    ) -> Result<u64> {
        if observations.is_empty() {
            return Ok(0);
        }
        let token_ids: Vec<Uuid> = observations.iter().map(|row| row.0).collect();
        let used_at: Vec<DateTime<Utc>> = observations.iter().map(|row| row.1).collect();
        let result = sqlx::query_file!(
            "src/sql/db/personal_access_tokens/touch_last_used.sql",
            &token_ids,
            &used_at
        )
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected())
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
        group_scope_all: row.group_scope_all,
        group_ids: row.group_ids,
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        last_used_precision: row.last_used_precision,
        revoked_at: row.revoked_at,
        rotated_from_token_id: row.rotated_from_token_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    use super::{PersonalAccessTokenRecord, PersonalAccessTokenSnapshot};

    fn record() -> PersonalAccessTokenRecord {
        let at = Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap();
        PersonalAccessTokenRecord {
            id: Uuid::nil(),
            user_id: 7,
            name: "build".to_string(),
            token_hash: "hash".to_string(),
            display_prefix: "ctx_pat_prefix".to_string(),
            scopes: vec!["search".to_string()],
            group_scope_all: false,
            group_ids: vec![1, 2],
            expires_at: at,
            last_used_at: None,
            last_used_precision: None,
            revoked_at: None,
            rotated_from_token_id: None,
            created_at: at,
            updated_at: at,
        }
    }

    #[test]
    fn snapshot_matches_the_configuration_it_was_read_from() {
        let record = record();
        assert!(PersonalAccessTokenSnapshot::from_record(&record).matches(&record));
    }

    #[test]
    fn snapshot_rejects_every_configuration_or_revocation_change() {
        let record = record();
        let snapshot = PersonalAccessTokenSnapshot::from_record(&record);

        let mut renamed = record.clone();
        renamed.name = "other".to_string();
        assert!(!snapshot.matches(&renamed));

        let mut rescoped = record.clone();
        rescoped.scopes = vec!["workspace".to_string()];
        assert!(!snapshot.matches(&rescoped));

        let mut widened = record.clone();
        widened.group_scope_all = true;
        widened.group_ids.clear();
        assert!(!snapshot.matches(&widened));

        let mut narrowed = record.clone();
        narrowed.group_ids = vec![1];
        assert!(!snapshot.matches(&narrowed));

        let mut expired = record.clone();
        expired.expires_at = record.expires_at + chrono::Duration::days(1);
        assert!(!snapshot.matches(&expired));

        let mut revoked = record.clone();
        revoked.revoked_at = Some(record.updated_at);
        assert!(!snapshot.matches(&revoked));
    }

    #[test]
    fn snapshot_ignores_telemetry_and_storage_only_changes() {
        let record = record();
        let snapshot = PersonalAccessTokenSnapshot::from_record(&record);

        let mut used = record.clone();
        used.last_used_at = Some(record.updated_at);
        used.last_used_precision = Some("exact".to_string());
        used.updated_at = record.updated_at + chrono::Duration::seconds(1);
        assert!(snapshot.matches(&used));
    }
}
