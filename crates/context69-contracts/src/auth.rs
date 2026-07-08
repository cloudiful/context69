use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::MembershipRole;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthLoginRequest {
    pub login_name: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthUserResponse {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub is_admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<DateTime<Utc>>,
    pub personal_group_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_group_role: Option<MembershipRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in_secs: u64,
    pub user: AuthUserResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthMeResponse {
    pub user: AuthUserResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PersonalAccessTokenScope {
    Search,
    Workspace,
    Library,
    Sources,
    Settings,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatePersonalAccessTokenRequest {
    pub name: String,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub expires_in_days: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PersonalAccessTokenResponse {
    pub token_id: Uuid,
    pub name: String,
    pub display_prefix: String,
    pub scopes: Vec<PersonalAccessTokenScope>,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatePersonalAccessTokenResponse {
    pub access_token: String,
    pub token: PersonalAccessTokenResponse,
}
