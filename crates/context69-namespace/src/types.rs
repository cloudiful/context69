use chrono::{DateTime, Utc};
use context69_contracts::{GroupKind, MembershipRole, Visibility};

#[derive(Debug, Clone)]
pub struct NamespaceActor {
    pub user_id: i64,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct CreateGroupInput {
    pub parent_group_key: Option<String>,
    pub group_key: String,
    pub name: String,
    pub visibility: Visibility,
    pub kind: Option<GroupKind>,
}

#[derive(Debug, Clone)]
pub struct UpdateGroupInput {
    pub name: Option<String>,
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone)]
pub struct CreateProjectInput {
    pub project_key: String,
    pub name: String,
    pub visibility: Visibility,
}

#[derive(Debug, Clone)]
pub struct UpdateProjectInput {
    pub name: Option<String>,
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone)]
pub struct MoveProjectInput {
    pub target_group_key: String,
}

#[derive(Debug, Clone)]
pub struct UpsertMembershipInput {
    pub login_name: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone)]
pub struct GroupRecord {
    pub id: i64,
    pub parent_group_id: Option<i64>,
    pub parent_group_key: Option<String>,
    pub group_key: String,
    pub name: String,
    pub visibility: Visibility,
    pub kind: GroupKind,
    pub owner_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_role: Option<MembershipRole>,
}

#[derive(Debug, Clone)]
pub struct ProjectRecord {
    pub id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub project_key: String,
    pub name: String,
    pub visibility: Visibility,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_role: Option<MembershipRole>,
}

#[derive(Debug, Clone)]
pub struct NamespaceMemberRecord {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone)]
pub struct PersonalGroupRecord {
    pub group_id: i64,
    pub group_key: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone)]
pub struct AccessScope {
    pub user_id: Option<i64>,
    pub include_public: bool,
    pub private_project_ids: Vec<i64>,
    pub group_key: Option<String>,
    pub project_key: Option<String>,
}
