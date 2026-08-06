use chrono::{DateTime, Utc};
use context69_contracts::{GroupKind, MembershipRole, Visibility};

#[derive(Debug, Clone)]
pub struct NamespaceActor {
    pub user_id: i64,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct CreateGroupInput {
    pub parent_group_path: Option<String>,
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
pub struct MoveGroupInput {
    pub target_parent_group_path: Option<String>,
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
    pub group_path: String,
    pub parent_group_path: Option<String>,
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
pub struct NamespaceMemberRecord {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone)]
pub struct PersonalGroupRecord {
    pub group_id: i64,
    pub group_path: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone)]
pub struct AccessScope {
    pub user_id: Option<i64>,
    pub include_public: bool,
    pub private_group_ids: Vec<i64>,
    pub group_path: Option<String>,
    /// Resolved id of the group that a scoped request is narrowed to.
    /// `None` while `group_path` is `Some` means the group no longer exists.
    pub scoped_group_id: Option<i64>,
}
