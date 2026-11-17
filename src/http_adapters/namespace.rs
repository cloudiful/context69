use async_trait::async_trait;
use chrono::Utc;
use context69_contracts::{
    CreateGroupRequest, GroupMemberPageResponse, GroupMemberResponse, GroupPageResponse,
    GroupResponse, GroupSearchQuery, MemberPageQuery, MoveGroupRequest, NamespacePageQuery,
    SortDirection, UpdateGroupRequest, UpsertMembershipRequest, UserDirectoryEntryResponse,
};
use context69_http_support::AuthenticatedUser;
use context69_namespace::{PageRequest, PageSort};
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

    async fn require_allowed_group(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
    ) -> anyhow::Result<()> {
        let Some(restriction) = actor
            .token_group_scope
            .as_ref()
            .filter(|scope| scope.is_restricted())
        else {
            return Ok(());
        };
        if let Some(group) = self
            .service
            .get_group_for_user(actor.user_id, group_path, None)
            .await?
        {
            crate::authorization::require_group_allowed(Some(restriction), group.id)?;
        }
        Ok(())
    }
}

#[async_trait]
impl NamespaceApi for NamespaceApiAdapter {
    async fn list_groups_for_user(
        &self,
        actor: &AuthenticatedUser,
        query: &NamespacePageQuery,
    ) -> anyhow::Result<GroupPageResponse> {
        let page = self
            .service
            .list_groups_for_user(
                actor.user_id,
                &page_request(query),
                actor.token_group_scope.as_ref(),
            )
            .await?;
        Ok(group_page_response(page))
    }

    async fn search_groups_for_user(
        &self,
        actor: &AuthenticatedUser,
        query: &GroupSearchQuery,
    ) -> anyhow::Result<Vec<GroupResponse>> {
        Ok(self
            .service
            .search_groups_for_user(
                actor.user_id,
                &query.query,
                query.limit,
                actor.token_group_scope.as_ref(),
            )
            .await?
            .into_iter()
            .map(group_response)
            .collect())
    }

    async fn list_child_groups_for_user(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        query: &NamespacePageQuery,
    ) -> anyhow::Result<GroupPageResponse> {
        let page = self
            .service
            .list_child_groups_for_user(
                actor.user_id,
                group_path,
                &page_request(query),
                actor.token_group_scope.as_ref(),
            )
            .await?;
        Ok(group_page_response(page))
    }

    async fn get_group_for_user(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
    ) -> anyhow::Result<Option<GroupResponse>> {
        Ok(self
            .service
            .get_group_for_user(actor.user_id, group_path, actor.token_group_scope.as_ref())
            .await?
            .map(group_response))
    }

    async fn create_group(
        &self,
        actor: &AuthenticatedUser,
        request: &CreateGroupRequest,
    ) -> anyhow::Result<GroupResponse> {
        crate::authorization::require_unrestricted(actor.token_group_scope.as_ref())?;
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
        self.require_allowed_group(actor, group_path).await?;
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
        crate::authorization::require_unrestricted(actor.token_group_scope.as_ref())?;
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
        crate::authorization::require_unrestricted(actor.token_group_scope.as_ref())?;
        self.service
            .delete_group(&to_user_record(actor), group_path)
            .await
    }

    async fn list_group_members(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        query: &MemberPageQuery,
    ) -> anyhow::Result<GroupMemberPageResponse> {
        self.require_allowed_group(actor, group_path).await?;
        let page = self
            .service
            .list_group_members(
                &to_user_record(actor),
                group_path,
                &member_page_request(query),
            )
            .await?;
        Ok(group_member_page_response(page))
    }

    async fn upsert_group_member(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        request: &UpsertMembershipRequest,
    ) -> anyhow::Result<()> {
        // Membership changes inherit down the subtree, so a group-restricted
        // token cannot prove the change stays inside its exact group set.
        crate::authorization::require_unrestricted(actor.token_group_scope.as_ref())?;
        self.require_allowed_group(actor, group_path).await?;
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
        crate::authorization::require_unrestricted(actor.token_group_scope.as_ref())?;
        self.require_allowed_group(actor, group_path).await?;
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
        crate::authorization::require_unrestricted(actor.token_group_scope.as_ref())?;
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

pub(crate) fn page_request(query: &NamespacePageQuery) -> PageRequest {
    PageRequest {
        page: query.page,
        page_size: query.page_size,
        query: query.query.clone().unwrap_or_default(),
        sort: query.sort_by.map(|sort_by| PageSort {
            column: sort_by.as_str(),
            direction: query.sort_direction.unwrap_or(SortDirection::Desc),
        }),
        visibility: query.visibility,
        kind: query.kind,
    }
}

fn member_page_request(query: &MemberPageQuery) -> PageRequest {
    PageRequest {
        page: query.page,
        page_size: query.page_size,
        query: query.query.clone().unwrap_or_default(),
        sort: query.sort_by.map(|sort_by| PageSort {
            column: sort_by.as_str(),
            direction: query.sort_direction.unwrap_or(SortDirection::Desc),
        }),
        visibility: None,
        kind: None,
    }
}

pub(crate) fn group_page_response(
    page: context69_namespace::Page<GroupRecord>,
) -> GroupPageResponse {
    GroupPageResponse {
        items: page.items.into_iter().map(group_response).collect(),
        pagination: page.pagination,
    }
}

fn group_member_page_response(
    page: context69_namespace::Page<NamespaceMemberRecord>,
) -> GroupMemberPageResponse {
    GroupMemberPageResponse {
        items: page.items.into_iter().map(group_member_response).collect(),
        pagination: page.pagination,
    }
}
