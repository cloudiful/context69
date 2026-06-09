use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
