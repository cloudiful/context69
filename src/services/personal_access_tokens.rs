use std::collections::BTreeSet;

use anyhow::{Context, Result, anyhow};
use chrono::{Duration as ChronoDuration, Utc};
use uuid::Uuid;

use crate::{
    contracts::PersonalAccessTokenScope,
    db::{Database, PersonalAccessTokenRecord},
    services::{auth::AuthService, token_utils::hash_token},
};

const PERSONAL_ACCESS_TOKEN_PREFIX: &str = "ctx_pat_";
const ALLOWED_EXPIRY_DAYS: [u16; 4] = [7, 30, 90, 365];

#[derive(Debug, Clone)]
pub struct PersonalAccessTokenView {
    pub token_id: Uuid,
    pub name: String,
    pub display_prefix: String,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub expires_at: chrono::DateTime<Utc>,
    pub last_used_at: Option<chrono::DateTime<Utc>>,
    pub revoked_at: Option<chrono::DateTime<Utc>>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreatedPersonalAccessToken {
    pub access_token: String,
    pub token: PersonalAccessTokenView,
}

#[derive(Debug, Clone)]
pub struct VerifiedPersonalAccessToken {
    pub token_id: Uuid,
    pub scopes: BTreeSet<PersonalAccessTokenScope>,
    pub session: crate::services::auth::AuthSession,
}

#[derive(Clone)]
pub struct PersonalAccessTokenService {
    db: Database,
    auth: AuthService,
}

impl PersonalAccessTokenService {
    pub fn new(db: Database, auth: AuthService) -> Self {
        Self { db, auth }
    }

    pub async fn create_for_user(
        &self,
        user_id: i64,
        name: &str,
        scopes: &[PersonalAccessTokenScope],
        expires_in_days: u16,
    ) -> Result<CreatedPersonalAccessToken> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err(anyhow!("token name must not be empty"));
        }
        if scopes.is_empty() {
            return Err(anyhow!("at least one scope is required"));
        }
        if !ALLOWED_EXPIRY_DAYS.contains(&expires_in_days) {
            return Err(anyhow!(
                "expires_in_days must be one of {}",
                ALLOWED_EXPIRY_DAYS
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let access_token = new_personal_access_token();
        let expires_at = Utc::now() + ChronoDuration::days(i64::from(expires_in_days));
        let scope_strings = scopes
            .iter()
            .copied()
            .map(scope_to_string)
            .collect::<Vec<_>>();
        let display_prefix = access_token.chars().take(18).collect::<String>();
        let record = self
            .db
            .insert_personal_access_token(&crate::db::NewPersonalAccessToken {
                id: Uuid::new_v4(),
                user_id,
                name: trimmed_name.to_string(),
                token_hash: hash_token(&access_token),
                display_prefix,
                scopes: scope_strings,
                expires_at,
            })
            .await?;
        Ok(CreatedPersonalAccessToken {
            access_token,
            token: token_view_from_record(record)?,
        })
    }

    pub async fn list_for_user(&self, user_id: i64) -> Result<Vec<PersonalAccessTokenView>> {
        self.db
            .list_personal_access_tokens(user_id)
            .await?
            .into_iter()
            .map(token_view_from_record)
            .collect()
    }

    pub async fn revoke_for_user(&self, user_id: i64, token_id: Uuid) -> Result<()> {
        self.db
            .revoke_personal_access_token(token_id, user_id)
            .await?
            .context("personal access token not found")?;
        Ok(())
    }

    pub async fn verify(&self, token: &str) -> Result<VerifiedPersonalAccessToken> {
        let record = self
            .db
            .get_personal_access_token_by_hash(&hash_token(token))
            .await?
            .context("invalid personal access token")?;
        validate_personal_access_token_record(&record)?;
        let scopes = record
            .scopes
            .iter()
            .map(|scope| parse_scope(scope))
            .collect::<Result<BTreeSet<_>>>()?;
        let session = self.auth.session_for_user_id(record.user_id).await?;
        Ok(VerifiedPersonalAccessToken {
            token_id: record.id,
            scopes,
            session,
        })
    }

    pub async fn touch_last_used(&self, token_id: Uuid) -> Result<()> {
        self.db
            .touch_personal_access_token_last_used(token_id)
            .await
    }
}

pub fn scope_to_string(scope: PersonalAccessTokenScope) -> String {
    match scope {
        PersonalAccessTokenScope::Search => "search",
        PersonalAccessTokenScope::Workspace => "workspace",
        PersonalAccessTokenScope::Library => "library",
        PersonalAccessTokenScope::Sources => "sources",
        PersonalAccessTokenScope::Settings => "settings",
        PersonalAccessTokenScope::Admin => "admin",
    }
    .to_string()
}

fn parse_scope(value: &str) -> Result<PersonalAccessTokenScope> {
    match value {
        "search" => Ok(PersonalAccessTokenScope::Search),
        "workspace" => Ok(PersonalAccessTokenScope::Workspace),
        "library" => Ok(PersonalAccessTokenScope::Library),
        "sources" => Ok(PersonalAccessTokenScope::Sources),
        "settings" => Ok(PersonalAccessTokenScope::Settings),
        "admin" => Ok(PersonalAccessTokenScope::Admin),
        _ => Err(anyhow!("unknown personal access token scope: {value}")),
    }
}

fn token_view_from_record(record: PersonalAccessTokenRecord) -> Result<PersonalAccessTokenView> {
    Ok(PersonalAccessTokenView {
        token_id: record.id,
        name: record.name,
        display_prefix: record.display_prefix,
        scopes: record
            .scopes
            .iter()
            .map(|scope| parse_scope(scope))
            .collect::<Result<Vec<_>>>()?,
        expires_at: record.expires_at,
        last_used_at: record.last_used_at,
        revoked_at: record.revoked_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn validate_personal_access_token_record(record: &PersonalAccessTokenRecord) -> Result<()> {
    if record.revoked_at.is_some() {
        return Err(anyhow!("personal access token has been revoked"));
    }
    if record.expires_at <= Utc::now() {
        return Err(anyhow!("personal access token has expired"));
    }
    Ok(())
}

fn new_personal_access_token() -> String {
    format!(
        "{PERSONAL_ACCESS_TOKEN_PREFIX}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

pub fn is_personal_access_token(token: &str) -> bool {
    token.starts_with(PERSONAL_ACCESS_TOKEN_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::{ALLOWED_EXPIRY_DAYS, is_personal_access_token, scope_to_string};
    use crate::contracts::PersonalAccessTokenScope;

    #[test]
    fn allowed_expiry_days_are_stable() {
        assert_eq!(ALLOWED_EXPIRY_DAYS, [7, 30, 90, 365]);
    }

    #[test]
    fn detects_pat_prefix() {
        assert!(is_personal_access_token("ctx_pat_abc"));
        assert!(!is_personal_access_token("Bearer abc"));
    }

    #[test]
    fn scope_strings_match_openapi_shape() {
        assert_eq!(scope_to_string(PersonalAccessTokenScope::Search), "search");
        assert_eq!(scope_to_string(PersonalAccessTokenScope::Admin), "admin");
    }
}
