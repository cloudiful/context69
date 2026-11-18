use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;

use crate::domain_errors::DomainError;
use chrono::{Duration as ChronoDuration, Utc};
use uuid::Uuid;

use crate::{
    contracts::{PersonalAccessTokenLastUsedPrecision, PersonalAccessTokenScope, TokenGroupScope},
    db::{Database, PersonalAccessTokenRecord},
    pagination::PageBounds,
    services::{auth::AuthService, token_utils::hash_token},
};

mod last_used_recorder;

pub use last_used_recorder::LastUsedRecorder;

const PERSONAL_ACCESS_TOKEN_PREFIX: &str = "ctx_pat_";
const ALLOWED_EXPIRY_DAYS: [u16; 4] = [7, 30, 90, 365];

#[derive(Debug, Clone)]
pub struct PersonalAccessTokenView {
    pub token_id: Uuid,
    pub name: String,
    pub display_prefix: String,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub group_scope: TokenGroupScope,
    pub expires_at: chrono::DateTime<Utc>,
    pub last_used_at: Option<chrono::DateTime<Utc>>,
    pub last_used_precision: Option<PersonalAccessTokenLastUsedPrecision>,
    pub revoked_at: Option<chrono::DateTime<Utc>>,
    pub rotated_from_token_id: Option<Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PersonalAccessTokenPage {
    pub items: Vec<PersonalAccessTokenView>,
    pub pagination: context69_contracts::Pagination,
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
    pub group_scope: TokenGroupScope,
    pub telemetry_due: bool,
    pub session: crate::services::auth::AuthSession,
}

#[derive(Clone)]
pub struct PersonalAccessTokenService {
    db: Database,
    auth: AuthService,
    usage: Arc<LastUsedRecorder>,
}

impl PersonalAccessTokenService {
    pub fn new(db: Database, auth: AuthService, usage: Arc<LastUsedRecorder>) -> Self {
        Self { db, auth, usage }
    }

    pub async fn create_for_user(
        &self,
        user_id: i64,
        is_admin: bool,
        name: &str,
        scopes: &[PersonalAccessTokenScope],
        group_scope: Option<&TokenGroupScope>,
        expires_in_days: u16,
    ) -> Result<CreatedPersonalAccessToken> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err(DomainError::invalid_argument("token name must not be empty").into());
        }
        validate_expiry_days(expires_in_days)?;
        let group_scope = group_scope.cloned().unwrap_or(TokenGroupScope::All);
        self.validate_configuration(user_id, is_admin, scopes, &group_scope)
            .await?;

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
                group_scope_all: group_scope_all(&group_scope),
                group_ids: group_ids(&group_scope),
                expires_at,
                rotated_from_token_id: None,
            })
            .await?;
        Ok(CreatedPersonalAccessToken {
            access_token,
            token: token_view_from_record(record)?,
        })
    }

    /// Update the editable configuration of one token owned by the user.
    ///
    /// The request omits fields it does not change. Expiry and revocation are
    /// never touched, so an expired or revoked row keeps its status.
    pub async fn update_for_user(
        &self,
        user_id: i64,
        is_admin: bool,
        token_id: Uuid,
        name: Option<&str>,
        scopes: Option<&[PersonalAccessTokenScope]>,
        group_scope: Option<&TokenGroupScope>,
    ) -> Result<PersonalAccessTokenView> {
        let record = self
            .db
            .get_personal_access_token(token_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("personal access token not found"))?;
        let expected = crate::db::PersonalAccessTokenSnapshot::from_record(&record);
        let current_scopes = parse_scopes(&record.scopes)?;
        let new_scopes = scopes.unwrap_or(&current_scopes);
        let current_group_scope = group_scope_from_record(&record);
        let new_group_scope = group_scope.unwrap_or(&current_group_scope);
        let new_name = name.map(str::trim).unwrap_or_else(|| record.name.as_str());

        if new_name.is_empty() {
            return Err(DomainError::invalid_argument("token name must not be empty").into());
        }
        self.validate_configuration(user_id, is_admin, new_scopes, new_group_scope)
            .await?;

        let scope_strings = new_scopes
            .iter()
            .copied()
            .map(scope_to_string)
            .collect::<Vec<_>>();
        let updated = self
            .db
            .update_personal_access_token(
                token_id,
                user_id,
                &expected,
                &crate::db::PersonalAccessTokenUpdate {
                    name: new_name.to_string(),
                    scopes: scope_strings,
                    group_scope_all: group_scope_all(new_group_scope),
                    group_ids: group_ids(new_group_scope),
                },
            )
            .await?;
        match updated {
            crate::db::PersonalAccessTokenMutation::Applied(record) => {
                token_view_from_record(*record)
            }
            crate::db::PersonalAccessTokenMutation::Stale => Err(stale_token_conflict()),
            crate::db::PersonalAccessTokenMutation::NotFound => {
                Err(DomainError::not_found("personal access token not found").into())
            }
        }
    }

    /// Generate a replacement for an active, expired, or revoked token.
    ///
    /// The old row keeps its history and is revoked; the replacement inherits
    /// the name, scopes, and group range, resets usage, and is created only
    /// after the current authorization was revalidated.
    pub async fn rotate_for_user(
        &self,
        user_id: i64,
        is_admin: bool,
        token_id: Uuid,
        expires_in_days: u16,
    ) -> Result<CreatedPersonalAccessToken> {
        validate_expiry_days(expires_in_days)?;
        let record = self
            .db
            .get_personal_access_token(token_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("personal access token not found"))?;
        let expected = crate::db::PersonalAccessTokenSnapshot::from_record(&record);
        let scopes = parse_scopes(&record.scopes)?;
        let group_scope = group_scope_from_record(&record);
        self.validate_configuration(user_id, is_admin, &scopes, &group_scope)
            .await?;

        let access_token = new_personal_access_token();
        let expires_at = Utc::now() + ChronoDuration::days(i64::from(expires_in_days));
        let display_prefix = access_token.chars().take(18).collect::<String>();
        let replacement = self
            .db
            .rotate_personal_access_token(
                token_id,
                user_id,
                &expected,
                &crate::db::NewPersonalAccessTokenSecret {
                    id: Uuid::new_v4(),
                    user_id,
                    token_hash: hash_token(&access_token),
                    display_prefix,
                    expires_at,
                    rotated_from_token_id: token_id,
                },
            )
            .await?;
        let replacement = match replacement {
            crate::db::PersonalAccessTokenMutation::Applied(record) => *record,
            crate::db::PersonalAccessTokenMutation::Stale => return Err(stale_token_conflict()),
            crate::db::PersonalAccessTokenMutation::NotFound => {
                return Err(DomainError::not_found("personal access token not found").into());
            }
        };
        Ok(CreatedPersonalAccessToken {
            access_token,
            token: token_view_from_record(replacement)?,
        })
    }

    /// Validate a requested scope and group configuration against the owner.
    ///
    /// A restricted token can never carry a global Settings or Admin scope,
    /// every named group must currently resolve for the owner, and a user
    /// without admin rights can never grant the Admin scope.
    async fn validate_configuration(
        &self,
        user_id: i64,
        is_admin: bool,
        scopes: &[PersonalAccessTokenScope],
        group_scope: &TokenGroupScope,
    ) -> Result<()> {
        if scopes.is_empty() {
            return Err(DomainError::invalid_argument("at least one scope is required").into());
        }
        if scopes.contains(&PersonalAccessTokenScope::Admin) && !is_admin {
            return Err(DomainError::invalid_argument(
                "admin scope requires an administrator account",
            )
            .into());
        }
        if let Some(group_ids) = group_scope.restricted_group_ids() {
            if group_ids.is_empty() {
                return Err(DomainError::invalid_argument(
                    "a restricted token must name at least one group",
                )
                .into());
            }
            if scopes.iter().any(|scope| {
                matches!(
                    scope,
                    PersonalAccessTokenScope::Settings | PersonalAccessTokenScope::Admin
                )
            }) {
                return Err(DomainError::invalid_argument(
                    "settings and admin scopes are not available to a group-restricted token",
                )
                .into());
            }
            let mut requested = group_ids.to_vec();
            requested.sort_unstable();
            requested.dedup();
            let accessible = self
                .db
                .resolve_accessible_group_ids(user_id, &requested)
                .await?;
            if accessible != requested {
                return Err(DomainError::invalid_argument(
                    "token group scope names a group the owner cannot access",
                )
                .into());
            }
        }
        Ok(())
    }

    pub async fn list_for_user(&self, user_id: i64) -> Result<Vec<PersonalAccessTokenView>> {
        self.db
            .list_personal_access_tokens(user_id)
            .await?
            .into_iter()
            .map(token_view_from_record)
            .collect()
    }

    pub async fn list_page_for_user(
        &self,
        user_id: i64,
        page: u32,
        page_size: u32,
    ) -> Result<PersonalAccessTokenPage> {
        let bounds = PageBounds::new(page, page_size)?;
        let total = self.db.count_personal_access_tokens(user_id).await?;
        let items = self
            .db
            .list_personal_access_tokens_page(user_id, i64::from(bounds.page_size), bounds.offset)
            .await?
            .into_iter()
            .map(token_view_from_record)
            .collect::<Result<Vec<_>>>()?;
        Ok(PersonalAccessTokenPage {
            items,
            pagination: bounds.pagination(total)?,
        })
    }

    pub async fn revoke_for_user(&self, user_id: i64, token_id: Uuid) -> Result<()> {
        self.db
            .revoke_personal_access_token(token_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("personal access token not found"))?;
        Ok(())
    }

    pub async fn verify(&self, token: &str) -> Result<VerifiedPersonalAccessToken> {
        let record = self
            .db
            .get_personal_access_token_by_hash(&hash_token(token))
            .await?
            .ok_or_else(|| DomainError::unauthorized("invalid personal access token"))?;
        validate_personal_access_token_record(&record)?;
        let scopes = record
            .scopes
            .iter()
            .map(|scope| parse_scope(scope))
            .collect::<Result<BTreeSet<_>>>()?;
        let group_scope = group_scope_from_record(&record);
        let telemetry_due = true;
        let mut session = self.auth.session_for_user_id(record.user_id).await?;
        session.token_group_scope = Some(group_scope.clone());
        Ok(VerifiedPersonalAccessToken {
            token_id: record.id,
            scopes,
            group_scope,
            telemetry_due,
            session,
        })
    }

    /// Record that a token was used.
    ///
    /// The observation is coalesced with the other observations of the same
    /// token and written later, so the caller never waits for the write and
    /// never fails because of it. The captured instant is the authorization
    /// time, not the later flush time.
    pub fn record_last_used(&self, token_id: Uuid) {
        self.usage.record(token_id, Utc::now());
    }
}

fn stale_token_conflict() -> anyhow::Error {
    DomainError::conflict("personal access token changed; reload and retry").into()
}

fn validate_expiry_days(expires_in_days: u16) -> Result<()> {
    if !ALLOWED_EXPIRY_DAYS.contains(&expires_in_days) {
        return Err(DomainError::invalid_argument(format!(
            "expires_in_days must be one of {}",
            ALLOWED_EXPIRY_DAYS
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .into());
    }
    Ok(())
}

fn group_scope_all(group_scope: &TokenGroupScope) -> bool {
    group_scope.is_all()
}

fn group_ids(group_scope: &TokenGroupScope) -> Vec<i64> {
    group_scope.normalized_group_ids().unwrap_or_default()
}

fn group_scope_from_record(record: &PersonalAccessTokenRecord) -> TokenGroupScope {
    if record.group_scope_all {
        TokenGroupScope::All
    } else {
        let mut group_ids = record.group_ids.clone();
        group_ids.sort_unstable();
        group_ids.dedup();
        TokenGroupScope::Groups { group_ids }
    }
}

fn parse_scopes(scopes: &[String]) -> Result<Vec<PersonalAccessTokenScope>> {
    scopes.iter().map(|scope| parse_scope(scope)).collect()
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
        _ => Err(DomainError::invalid_argument(format!(
            "unknown personal access token scope: {value}"
        ))
        .into()),
    }
}

fn token_view_from_record(record: PersonalAccessTokenRecord) -> Result<PersonalAccessTokenView> {
    let group_scope = group_scope_from_record(&record);
    let last_used_precision = record
        .last_used_precision
        .as_deref()
        .map(parse_last_used_precision)
        .transpose()?;
    Ok(PersonalAccessTokenView {
        token_id: record.id,
        name: record.name,
        display_prefix: record.display_prefix,
        scopes: record
            .scopes
            .iter()
            .map(|scope| parse_scope(scope))
            .collect::<Result<Vec<_>>>()?,
        group_scope,
        expires_at: record.expires_at,
        last_used_at: record.last_used_at,
        last_used_precision,
        revoked_at: record.revoked_at,
        rotated_from_token_id: record.rotated_from_token_id,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn parse_last_used_precision(value: &str) -> Result<PersonalAccessTokenLastUsedPrecision> {
    match value {
        "exact" => Ok(PersonalAccessTokenLastUsedPrecision::Exact),
        "legacy_day" => Ok(PersonalAccessTokenLastUsedPrecision::LegacyDay),
        other => Err(DomainError::internal(format!(
            "unknown personal access token last-used precision: {other}"
        ))
        .into()),
    }
}

fn validate_personal_access_token_record(record: &PersonalAccessTokenRecord) -> Result<()> {
    if record.revoked_at.is_some() {
        return Err(DomainError::unauthorized("personal access token has been revoked").into());
    }
    if record.expires_at <= Utc::now() {
        return Err(DomainError::unauthorized("personal access token has expired").into());
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

#[cfg(test)]
mod last_used_recorder_tests;
