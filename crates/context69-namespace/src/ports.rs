use anyhow::Result;
use async_trait::async_trait;

use crate::{
    AccessScope, CreateGroupInput, GroupRecord, MoveGroupInput, NamespaceActor,
    NamespaceMemberRecord, UpdateGroupInput, UpsertMembershipInput,
};
use context69_contracts::Pagination;

#[derive(Debug, Clone)]
pub struct PageRequest {
    pub page: u32,
    pub page_size: u32,
    pub query: String,
}

#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub pagination: Pagination,
}

#[async_trait]
pub trait NamespaceRepository: Send + Sync {
    async fn list_groups_for_user(
        &self,
        user_id: i64,
        request: &PageRequest,
    ) -> Result<Page<GroupRecord>>;
    async fn search_groups_for_user(
        &self,
        user_id: i64,
        query: &str,
        limit: u32,
    ) -> Result<Vec<GroupRecord>>;
    async fn get_group_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Option<GroupRecord>>;
    async fn create_group(
        &self,
        actor: &NamespaceActor,
        request: &CreateGroupInput,
    ) -> Result<GroupRecord>;
    async fn update_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord>;
    async fn move_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &MoveGroupInput,
    ) -> Result<GroupRecord>;
    async fn delete_group(&self, actor: &NamespaceActor, group_path: &str) -> Result<()>;
    async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &PageRequest,
    ) -> Result<Page<NamespaceMemberRecord>>;
    async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()>;
    async fn delete_group_member(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        login_name: &str,
    ) -> Result<()>;
    async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
        request: &PageRequest,
    ) -> Result<Page<GroupRecord>>;
    async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope>;
}
