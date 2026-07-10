use async_trait::async_trait;
use chrono::Utc;
use context69_contracts::{
    CreateGroupRequest, GroupMemberResponse, GroupResponse, MoveGroupRequest, UpdateGroupRequest,
    UpsertMembershipRequest, UserDirectoryEntryResponse,
};
use context69_http_support::AuthenticatedUser;
use context69_namespace_http::{NamespaceApi, UserDirectoryApi};

use crate::{
    domain::{GroupRecord, NamespaceMemberRecord, UserRecord},
    services::{auth::AuthService, namespace::NamespaceService},
};

#[derive(Clone)]
pub struct NamespaceApiAdapter {
    service: NamespaceService,
}

impl NamespaceApiAdapter {
    pub fn new(service: NamespaceService) -> Self {
        Self { service }
    }
}

#[async_trait]
impl NamespaceApi for NamespaceApiAdapter {
    async fn list_groups_for_user(&self, user_id: i64) -> anyhow::Result<Vec<GroupResponse>> {
        Ok(self
            .service
            .list_groups_for_user(user_id)
            .await?
            .into_iter()
            .map(group_response)
            .collect())
    }

    async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> anyhow::Result<Vec<GroupResponse>> {
        Ok(self
            .service
            .list_child_groups_for_user(user_id, group_path)
            .await?
            .into_iter()
            .map(group_response)
            .collect())
    }

    async fn get_group_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> anyhow::Result<Option<GroupResponse>> {
        Ok(self
            .service
            .get_group_for_user(user_id, group_path)
            .await?
            .map(group_response))
    }

    async fn create_group(
        &self,
        actor: &AuthenticatedUser,
        request: &CreateGroupRequest,
    ) -> anyhow::Result<GroupResponse> {
        self.service
            .create_group(&to_user_record(actor), request)
            .await
            .map(group_response)
    }

    async fn update_group(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        request: &UpdateGroupRequest,
    ) -> anyhow::Result<GroupResponse> {
        self.service
            .update_group(&to_user_record(actor), group_path, request)
            .await
            .map(group_response)
    }

    async fn move_group(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        request: &MoveGroupRequest,
    ) -> anyhow::Result<GroupResponse> {
        self.service
            .move_group(&to_user_record(actor), group_path, request)
            .await
            .map(group_response)
    }

    async fn delete_group(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
    ) -> anyhow::Result<()> {
        self.service
            .delete_group(&to_user_record(actor), group_path)
            .await
    }

    async fn list_group_members(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
    ) -> anyhow::Result<Vec<GroupMemberResponse>> {
        Ok(self
            .service
            .list_group_members(&to_user_record(actor), group_path)
            .await?
            .into_iter()
            .map(group_member_response)
            .collect())
    }

    async fn upsert_group_member(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        request: &UpsertMembershipRequest,
    ) -> anyhow::Result<()> {
        self.service
            .upsert_group_member(&to_user_record(actor), group_path, request)
            .await
    }

    async fn delete_group_member(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        login_name: &str,
    ) -> anyhow::Result<()> {
        self.service
            .delete_group_member(&to_user_record(actor), group_path, login_name)
            .await
    }
}

#[derive(Clone)]
pub struct UserDirectoryApiAdapter {
    auth: AuthService,
}

impl UserDirectoryApiAdapter {
    pub fn new(auth: AuthService) -> Self {
        Self { auth }
    }
}

#[async_trait]
impl UserDirectoryApi for UserDirectoryApiAdapter {
    async fn search_user_directory(
        &self,
        actor: &AuthenticatedUser,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<UserDirectoryEntryResponse>> {
        Ok(self
            .auth
            .search_user_directory(&to_user_record(actor), query, limit)
            .await?
            .into_iter()
            .map(|user| UserDirectoryEntryResponse {
                user_id: user.id,
                login_name: user.login_name,
                display_name: user.display_name,
            })
            .collect())
    }
}

fn to_user_record(user: &AuthenticatedUser) -> UserRecord {
    UserRecord {
        id: user.user_id,
        login_name: user.login_name.clone(),
        display_name: user.display_name.clone(),
        password_hash: String::new(),
        is_admin: user.is_admin,
        disabled_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn group_response(group: GroupRecord) -> GroupResponse {
    GroupResponse {
        group_id: group.id,
        group_key: group.group_key,
        group_path: Some(group.group_path),
        parent_group_path: group.parent_group_path,
        name: group.name,
        visibility: group.visibility,
        kind: group.kind,
        current_role: group.current_role,
        created_at: group.created_at,
        updated_at: group.updated_at,
    }
}

fn group_member_response(member: NamespaceMemberRecord) -> GroupMemberResponse {
    GroupMemberResponse {
        user_id: member.user_id,
        login_name: member.login_name,
        display_name: member.display_name,
        role: member.role,
    }
}
