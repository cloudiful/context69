use anyhow::{Context, Result, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum_login::{AuthUser, AuthnBackend, UserId};
use chrono::Utc;
use uuid::Uuid;

use crate::{
    config::AuthConfig,
    contracts::{AuthUserResponse, MembershipRole},
    db::Database,
    domain::{AccessScope, PersonalGroupRecord, UserRecord},
};

pub const SESSION_COOKIE_NAME: &str = "context69_session_v2";
pub const AUTH_SESSION_DATA_KEY: &str = "context69.auth_session_v2";

#[derive(Clone)]
pub struct AuthService {
    db: Database,
    config: AuthConfig,
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub user: UserRecord,
    pub personal_group: PersonalGroupRecord,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub login_name: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct AuthPrincipal(pub AuthSession);

impl AuthUser for AuthPrincipal {
    type Id = i64;

    fn id(&self) -> Self::Id {
        self.0.user.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        self.0.user.password_hash.as_bytes()
    }
}

#[derive(Debug)]
pub struct AuthBackendError(anyhow::Error);

impl std::fmt::Display for AuthBackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for AuthBackendError {}

impl From<anyhow::Error> for AuthBackendError {
    fn from(value: anyhow::Error) -> Self {
        Self(value)
    }
}

impl AuthService {
    pub fn new(db: Database, config: AuthConfig) -> Result<Self> {
        Ok(Self { db, config })
    }

    pub async fn ensure_bootstrap_admin(&self) -> Result<()> {
        let Some(admin) = &self.config.bootstrap_admin else {
            return Ok(());
        };

        if self
            .db
            .get_user_by_login_name(&admin.login_name)
            .await?
            .is_some()
        {
            return Ok(());
        }

        let password_hash = hash_password(&admin.password)?;
        let user = self
            .db
            .create_user(
                admin.login_name.trim(),
                admin.display_name.trim(),
                &password_hash,
                true,
            )
            .await?;
        self.db.ensure_personal_group_for_user(&user).await?;
        Ok(())
    }

    pub async fn authenticate_credentials(
        &self,
        login_name: &str,
        password: &str,
    ) -> Result<AuthPrincipal> {
        let user = self
            .db
            .get_user_by_login_name(login_name.trim())
            .await?
            .context("invalid login or password")?;
        ensure_user_enabled(&user)?;
        verify_password(&user.password_hash, password)?;
        Ok(AuthPrincipal(self.session_for_user_id(user.id).await?))
    }

    pub async fn list_admin_users(&self, actor: &UserRecord) -> Result<Vec<UserRecord>> {
        require_admin(actor)?;
        self.db.list_users().await
    }

    pub async fn create_admin_user(
        &self,
        actor: &UserRecord,
        login_name: &str,
        display_name: &str,
        password: &str,
        is_admin: bool,
    ) -> Result<UserRecord> {
        require_admin(actor)?;

        let login_name = login_name.trim();
        let display_name = display_name.trim();
        let password = password.trim();
        if login_name.is_empty() {
            return Err(anyhow!("login_name must not be empty"));
        }
        if display_name.is_empty() {
            return Err(anyhow!("display_name must not be empty"));
        }
        if password.is_empty() {
            return Err(anyhow!("password must not be empty"));
        }

        let password_hash = hash_password(password)?;
        let user = self
            .db
            .create_user(login_name, display_name, &password_hash, is_admin)
            .await?;
        self.db.ensure_personal_group_for_user(&user).await?;
        Ok(user)
    }

    pub async fn update_admin_user(
        &self,
        actor: &UserRecord,
        login_name: &str,
        display_name: Option<&str>,
        is_admin: Option<bool>,
    ) -> Result<UserRecord> {
        require_admin(actor)?;

        let trimmed_display_name = display_name.map(str::trim);
        if let Some("") = trimmed_display_name {
            return Err(anyhow!("display_name must not be empty"));
        }

        self.db
            .update_user(login_name, trimmed_display_name, is_admin)
            .await?
            .context("user not found")
    }

    pub async fn reset_admin_user_password(
        &self,
        actor: &UserRecord,
        login_name: &str,
        password: &str,
    ) -> Result<UserRecord> {
        require_admin(actor)?;

        let password = password.trim();
        if password.is_empty() {
            return Err(anyhow!("password must not be empty"));
        }

        let password_hash = hash_password(password)?;
        self.db
            .update_user_password_hash(login_name, &password_hash)
            .await?
            .context("user not found")
    }

    pub async fn disable_admin_user(
        &self,
        actor: &UserRecord,
        login_name: &str,
    ) -> Result<UserRecord> {
        require_admin(actor)?;
        self.db
            .set_user_disabled_at(login_name, Some(Utc::now()))
            .await?
            .context("user not found")
    }

    pub async fn enable_admin_user(
        &self,
        actor: &UserRecord,
        login_name: &str,
    ) -> Result<UserRecord> {
        require_admin(actor)?;
        self.db
            .set_user_disabled_at(login_name, None)
            .await?
            .context("user not found")
    }

    pub async fn search_user_directory(
        &self,
        _actor: &UserRecord,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UserRecord>> {
        let trimmed_query = query.trim();
        let normalized_limit = limit.clamp(1, 20) as i64;
        if trimmed_query.is_empty() {
            return self.db.search_user_directory("", normalized_limit).await;
        }
        self.db
            .search_user_directory(trimmed_query, normalized_limit)
            .await
    }

    pub async fn session_for_user_id(&self, user_id: i64) -> Result<AuthSession> {
        let user = self
            .db
            .get_user_by_id(user_id)
            .await?
            .context("user not found")?;
        ensure_user_enabled(&user)?;
        let personal_group = self.db.ensure_personal_group_for_user(&user).await?;
        Ok(AuthSession {
            user,
            personal_group,
        })
    }

    pub async fn access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope> {
        self.db.resolve_access_scope(user_id, group_path).await
    }

    pub fn anonymous_mcp_enabled(&self) -> bool {
        self.config.anonymous_mcp_enabled
    }
}

impl AuthnBackend for AuthService {
    type User = AuthPrincipal;
    type Credentials = Credentials;
    type Error = AuthBackendError;

    async fn authenticate(
        &self,
        credentials: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        match self
            .authenticate_credentials(&credentials.login_name, &credentials.password)
            .await
        {
            Ok(principal) => Ok(Some(principal)),
            Err(error)
                if matches!(
                    error.to_string().as_str(),
                    "invalid login or password" | "user account is disabled"
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        match self.session_for_user_id(*user_id).await {
            Ok(session) => Ok(Some(AuthPrincipal(session))),
            Err(error)
                if matches!(
                    error.to_string().as_str(),
                    "user not found" | "user account is disabled"
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}

pub fn user_response(session: &AuthSession) -> AuthUserResponse {
    AuthUserResponse {
        user_id: session.user.id,
        login_name: session.user.login_name.clone(),
        display_name: session.user.display_name.clone(),
        is_admin: session.user.is_admin,
        disabled_at: session.user.disabled_at,
        personal_group_path: session.personal_group.group_path.clone(),
        personal_group_role: Some(MembershipRole::Owner),
    }
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes())
        .map_err(|error| anyhow!("failed to encode password salt: {error}"))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow!("failed to hash password: {error}"))?
        .to_string())
}

fn verify_password(password_hash: &str, password: &str) -> Result<()> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|error| anyhow!("invalid stored password hash: {error}"))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| anyhow!("invalid login or password"))
}

fn require_admin(actor: &UserRecord) -> Result<()> {
    if actor.is_admin {
        return Ok(());
    }
    Err(anyhow!("admin access required"))
}

fn ensure_user_enabled(user: &UserRecord) -> Result<()> {
    if user.disabled_at.is_some() {
        return Err(anyhow!("user account is disabled"));
    }
    Ok(())
}
