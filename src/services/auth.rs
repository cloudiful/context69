use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use chrono::{Duration as ChronoDuration, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    config::AuthConfig,
    contracts::{AuthTokenResponse, AuthUserResponse, MembershipRole},
    db::{Database, RefreshTokenRecord},
    domain::{AccessScope, PersonalGroupRecord, UserRecord},
};

#[derive(Clone)]
pub struct AuthService {
    db: Database,
    config: AuthConfig,
    encoding_keys: Arc<HashMap<String, EncodingKey>>,
    decoding_keys: Arc<HashMap<String, DecodingKey>>,
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub user: UserRecord,
    pub personal_group: PersonalGroupRecord,
}

#[derive(Debug, Clone)]
pub struct IssuedSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in_secs: u64,
    pub session: AuthSession,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AccessTokenClaims {
    sub: String,
    jti: String,
    iss: String,
    iat: i64,
    exp: i64,
}

impl AuthService {
    pub fn new(db: Database, config: AuthConfig) -> Result<Self> {
        let mut encoding_keys = HashMap::new();
        let mut decoding_keys = HashMap::new();
        for key in &config.signing_keys {
            encoding_keys.insert(
                key.kid.clone(),
                EncodingKey::from_secret(key.secret.as_bytes()),
            );
            decoding_keys.insert(
                key.kid.clone(),
                DecodingKey::from_secret(key.secret.as_bytes()),
            );
        }
        Ok(Self {
            db,
            config,
            encoding_keys: Arc::new(encoding_keys),
            decoding_keys: Arc::new(decoding_keys),
        })
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

    pub async fn verify_access_token(&self, token: &str) -> Result<AuthSession> {
        let header = decode_header(token).context("invalid token header")?;
        let kid = header.kid.context("missing token kid")?;
        let decoding_key = self.decoding_keys.get(&kid).context("unknown token kid")?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[self.config.issuer.as_str()]);
        let claims = decode::<AccessTokenClaims>(token, decoding_key, &validation)
            .context("invalid access token")?
            .claims;
        let user_id = claims
            .sub
            .parse::<i64>()
            .context("invalid access token subject")?;
        self.session_for_user_id(user_id).await
    }

    pub async fn login(&self, login_name: &str, password: &str) -> Result<IssuedSession> {
        let user = self
            .db
            .get_user_by_login_name(login_name.trim())
            .await?
            .context("invalid login or password")?;
        ensure_user_enabled(&user)?;
        verify_password(&user.password_hash, password)?;
        self.issue_session(user).await
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
        if matches!(trimmed_display_name, Some("")) {
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

    pub async fn refresh(&self, refresh_token: &str) -> Result<IssuedSession> {
        let current_token_hash = token_hash(refresh_token);
        let record = self
            .db
            .get_refresh_token_by_hash(&current_token_hash)
            .await?
            .context("invalid refresh token")?;
        validate_refresh_token_record(&record)?;
        let user = self
            .db
            .get_user_by_id(record.user_id)
            .await?
            .context("user not found")?;
        ensure_user_enabled(&user)?;

        let access_token = self.sign_access_token(user.id)?;
        let next_refresh_token = new_refresh_token();
        let next_refresh_hash = token_hash(&next_refresh_token);
        let next_expires_at = Utc::now()
            + ChronoDuration::from_std(self.config.refresh_token_ttl)
                .context("invalid refresh token ttl")?;
        self.db
            .rotate_refresh_token(
                record.id,
                &current_token_hash,
                Uuid::new_v4(),
                &next_refresh_hash,
                user.id,
                next_expires_at,
            )
            .await?;
        let session = self.session_for_user_id(user.id).await?;
        Ok(IssuedSession {
            access_token,
            refresh_token: next_refresh_token,
            expires_in_secs: self.config.access_token_ttl.as_secs(),
            session,
        })
    }

    pub async fn logout(&self, refresh_token: &str) -> Result<()> {
        self.db
            .revoke_refresh_token_by_hash(&token_hash(refresh_token))
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
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<AccessScope> {
        self.db
            .resolve_access_scope(user_id, group_key, project_key)
            .await
    }

    pub fn cookie_name(&self) -> &str {
        &self.config.refresh_cookie_name
    }

    pub fn refresh_cookie_secure(&self) -> bool {
        self.config.refresh_cookie_secure
    }

    pub fn refresh_token_ttl_secs(&self) -> i64 {
        self.config.refresh_token_ttl.as_secs() as i64
    }

    pub fn anonymous_mcp_enabled(&self) -> bool {
        self.config.anonymous_mcp_enabled
    }

    pub fn token_response(&self, issued: IssuedSession) -> AuthTokenResponse {
        AuthTokenResponse {
            access_token: issued.access_token,
            token_type: "Bearer".to_string(),
            expires_in_secs: issued.expires_in_secs,
            user: user_response(&issued.session),
        }
    }

    async fn issue_session(&self, user: UserRecord) -> Result<IssuedSession> {
        let access_token = self.sign_access_token(user.id)?;
        let refresh_token = new_refresh_token();
        let refresh_token_hash = token_hash(&refresh_token);
        let refresh_expires_at = Utc::now()
            + ChronoDuration::from_std(self.config.refresh_token_ttl)
                .context("invalid refresh token ttl")?;
        self.db
            .insert_refresh_token(
                Uuid::new_v4(),
                user.id,
                &refresh_token_hash,
                refresh_expires_at,
            )
            .await?;
        let session = self.session_for_user_id(user.id).await?;
        Ok(IssuedSession {
            access_token,
            refresh_token,
            expires_in_secs: self.config.access_token_ttl.as_secs(),
            session,
        })
    }

    fn sign_access_token(&self, user_id: i64) -> Result<String> {
        let now = Utc::now();
        let exp = now
            + ChronoDuration::from_std(self.config.access_token_ttl)
                .context("invalid access token ttl")?;
        let claims = AccessTokenClaims {
            sub: user_id.to_string(),
            jti: Uuid::new_v4().to_string(),
            iss: self.config.issuer.clone(),
            iat: now.timestamp(),
            exp: exp.timestamp(),
        };
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some(self.config.active_kid.clone());
        let encoding_key = self
            .encoding_keys
            .get(&self.config.active_kid)
            .context("missing active signing key")?;
        encode(&header, &claims, encoding_key).context("failed to sign access token")
    }
}

pub fn user_response(session: &AuthSession) -> AuthUserResponse {
    AuthUserResponse {
        user_id: session.user.id,
        login_name: session.user.login_name.clone(),
        display_name: session.user.display_name.clone(),
        is_admin: session.user.is_admin,
        disabled_at: session.user.disabled_at,
        personal_group_key: session.personal_group.group_key.clone(),
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

fn new_refresh_token() -> String {
    format!("rt_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let bytes = hasher.finalize();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_refresh_token_record(record: &RefreshTokenRecord) -> Result<()> {
    if record.revoked_at.is_some() {
        return Err(anyhow!("refresh token has been revoked"));
    }
    if record.replaced_by_token_id.is_some() {
        return Err(anyhow!("refresh token has been rotated"));
    }
    if record.expires_at <= Utc::now() {
        return Err(anyhow!("refresh token has expired"));
    }
    Ok(())
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
