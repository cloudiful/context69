use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::Pagination;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

impl std::str::FromStr for Visibility {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            other => Err(anyhow::anyhow!("unsupported visibility: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MembershipRole {
    Owner,
    Maintainer,
    Viewer,
}

impl MembershipRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Maintainer => "maintainer",
            Self::Viewer => "viewer",
        }
    }

    pub fn rank(self) -> i16 {
        match self {
            Self::Viewer => 1,
            Self::Maintainer => 2,
            Self::Owner => 3,
        }
    }
}

impl std::str::FromStr for MembershipRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "maintainer" => Ok(Self::Maintainer),
            "viewer" => Ok(Self::Viewer),
            other => Err(anyhow::anyhow!("unsupported membership role: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    Personal,
    Shared,
}

impl GroupKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Shared => "shared",
        }
    }
}

impl std::str::FromStr for GroupKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "personal" => Ok(Self::Personal),
            "shared" => Ok(Self::Shared),
            other => Err(anyhow::anyhow!("unsupported group kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_group_path: Option<String>,
    pub group_key: String,
    pub name: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub kind: Option<GroupKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateGroupRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MoveGroupRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_parent_group_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertMembershipRequest {
    pub login_name: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GroupResponse {
    pub group_id: i64,
    pub group_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_group_path: Option<String>,
    pub name: String,
    pub visibility: Visibility,
    pub kind: GroupKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_role: Option<MembershipRole>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GroupMemberResponse {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserDirectoryEntryResponse {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
}

fn default_page() -> u32 {
    1
}
fn default_page_size() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct NamespacePageQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GroupPageResponse {
    pub items: Vec<GroupResponse>,
    pub pagination: Pagination,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GroupMemberPageResponse {
    pub items: Vec<GroupMemberResponse>,
    pub pagination: Pagination,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct GroupSearchQuery {
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: u32,
}

const fn default_search_limit() -> u32 {
    20
}
