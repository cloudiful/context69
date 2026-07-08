use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use context69_contracts::{
    AuthLoginRequest, AuthMeResponse, AuthTokenResponse, AuthUserResponse,
    CreatePersonalAccessTokenRequest, CreatePersonalAccessTokenResponse,
    PersonalAccessTokenResponse, PersonalAccessTokenScope,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminUserResponse {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub is_admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateAdminUserRequest {
    pub login_name: String,
    pub display_name: String,
    pub password: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAdminUserRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_admin: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResetAdminUserPasswordRequest {
    pub password: String,
}
