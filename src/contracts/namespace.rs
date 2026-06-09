use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use context69_contracts::{
    GroupKind, GroupMemberResponse, GroupResponse, MembershipRole, ProjectMemberResponse,
    ProjectResponse, UserDirectoryEntryResponse, Visibility,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_group_key: Option<String>,
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
pub struct CreateProjectRequest {
    pub project_key: String,
    pub name: String,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateProjectRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MoveProjectRequest {
    pub target_group_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertMembershipRequest {
    pub login_name: String,
    pub role: MembershipRole,
}
