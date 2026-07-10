use anyhow::Result;

use crate::{
    AccessScope, CreateGroupInput, GroupRecord, MoveGroupInput, NamespaceActor,
    NamespaceMemberRecord, NamespaceRepository, UpdateGroupInput, UpsertMembershipInput,
};

#[derive(Clone)]
pub struct NamespaceService<R> {
    repository: R,
}

impl<R> NamespaceService<R> {
    pub fn new(repository: R) -> Self {
        Self { repository }
    }
}

impl<R: NamespaceRepository> NamespaceService<R> {
    pub async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupRecord>> {
        self.repository.list_groups_for_user(user_id).await
    }

    pub async fn get_group_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Option<GroupRecord>> {
        self.repository.get_group_for_user(user_id, group_path).await
    }

    pub async fn create_group(
        &self,
        actor: &NamespaceActor,
        request: &CreateGroupInput,
    ) -> Result<GroupRecord> {
        self.repository.create_group(actor, request).await
    }

    pub async fn update_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord> {
        self.repository
            .update_group(actor, group_path, request)
            .await
    }

    pub async fn move_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &MoveGroupInput,
    ) -> Result<GroupRecord> {
        self.repository.move_group(actor, group_path, request).await
    }

    pub async fn delete_group(&self, actor: &NamespaceActor, group_path: &str) -> Result<()> {
        self.repository.delete_group(actor, group_path).await
    }

    pub async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.repository.list_group_members(actor, group_path).await
    }

    pub async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        self.repository
            .upsert_group_member(actor, group_path, request)
            .await
    }

    pub async fn delete_group_member(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        login_name: &str,
    ) -> Result<()> {
        self.repository
            .delete_group_member(actor, group_path, login_name)
            .await
    }

    pub async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Vec<GroupRecord>> {
        self.repository
            .list_child_groups_for_user(user_id, group_path)
            .await
    }

    pub async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope> {
        self.repository.resolve_access_scope(user_id, group_path).await
    }
}
