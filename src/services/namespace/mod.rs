use anyhow::Result;
use context69_namespace::{
    CreateGroupInput, MoveGroupInput, NamespaceActor, NamespaceService as CoreNamespaceService,
    UpdateGroupInput, UpsertMembershipInput,
};

use crate::{
    contracts::{
        CreateGroupRequest, MoveGroupRequest, UpdateGroupRequest, UpsertMembershipRequest,
    },
    db::Database,
    domain::{AccessScope, GroupRecord, NamespaceMemberRecord, UserRecord},
};

mod adapters;

use self::adapters::DbNamespaceRepository;

#[derive(Clone)]
pub struct NamespaceService {
    inner: CoreNamespaceService<DbNamespaceRepository>,
}

impl NamespaceService {
    pub fn new(db: Database) -> Self {
        Self {
            inner: CoreNamespaceService::new(DbNamespaceRepository::new(db)),
        }
    }

    pub async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupRecord>> {
        self.inner.list_groups_for_user(user_id).await
    }

    pub async fn get_group_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Option<GroupRecord>> {
        self.inner.get_group_for_user(user_id, group_path).await
    }

    pub async fn create_group(
        &self,
        actor: &UserRecord,
        request: &CreateGroupRequest,
    ) -> Result<GroupRecord> {
        self.inner
            .create_group(&namespace_actor(actor), &create_group_input(request))
            .await
    }

    pub async fn update_group(
        &self,
        actor: &UserRecord,
        group_path: &str,
        request: &UpdateGroupRequest,
    ) -> Result<GroupRecord> {
        self.inner
            .update_group(
                &namespace_actor(actor),
                group_path,
                &update_group_input(request),
            )
            .await
    }

    pub async fn move_group(
        &self,
        actor: &UserRecord,
        group_path: &str,
        request: &MoveGroupRequest,
    ) -> Result<GroupRecord> {
        self.inner
            .move_group(
                &namespace_actor(actor),
                group_path,
                &move_group_input(request),
            )
            .await
    }

    pub async fn delete_group(&self, actor: &UserRecord, group_path: &str) -> Result<()> {
        self.inner
            .delete_group(&namespace_actor(actor), group_path)
            .await
    }

    pub async fn list_group_members(
        &self,
        actor: &UserRecord,
        group_path: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.inner
            .list_group_members(&namespace_actor(actor), group_path)
            .await
    }

    pub async fn upsert_group_member(
        &self,
        actor: &UserRecord,
        group_path: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()> {
        self.inner
            .upsert_group_member(
                &namespace_actor(actor),
                group_path,
                &upsert_membership_input(request),
            )
            .await
    }

    pub async fn delete_group_member(
        &self,
        actor: &UserRecord,
        group_path: &str,
        login_name: &str,
    ) -> Result<()> {
        self.inner
            .delete_group_member(&namespace_actor(actor), group_path, login_name)
            .await
    }

    pub async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Vec<GroupRecord>> {
        self.inner
            .list_child_groups_for_user(user_id, group_path)
            .await
    }

    pub async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope> {
        self.inner.resolve_access_scope(user_id, group_path).await
    }
}

fn namespace_actor(user: &UserRecord) -> NamespaceActor {
    NamespaceActor {
        user_id: user.id,
        is_admin: user.is_admin,
    }
}

fn create_group_input(request: &CreateGroupRequest) -> CreateGroupInput {
    CreateGroupInput {
        parent_group_path: request.parent_group_path.clone(),
        group_key: request.group_key.clone(),
        name: request.name.clone(),
        visibility: request.visibility,
        kind: request.kind,
    }
}

fn update_group_input(request: &UpdateGroupRequest) -> UpdateGroupInput {
    UpdateGroupInput {
        name: request.name.clone(),
        visibility: request.visibility,
    }
}

fn move_group_input(request: &MoveGroupRequest) -> MoveGroupInput {
    MoveGroupInput {
        target_parent_group_path: request.target_parent_group_path.clone(),
    }
}

fn upsert_membership_input(request: &UpsertMembershipRequest) -> UpsertMembershipInput {
    UpsertMembershipInput {
        login_name: request.login_name.clone(),
        role: request.role,
    }
}
