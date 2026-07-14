use anyhow::Result;
use async_trait::async_trait;
use context69_namespace::{
    AccessScope, CreateGroupInput, GroupRecord, MoveGroupInput, NamespaceActor,
    NamespaceMemberRecord, NamespaceRepository, Page, PageRequest, UpdateGroupInput,
    UpsertMembershipInput,
};

use crate::db::Database;

#[derive(Clone)]
pub(super) struct DbNamespaceRepository {
    db: Database,
}

impl DbNamespaceRepository {
    pub(super) fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl NamespaceRepository for DbNamespaceRepository {
    async fn list_groups_for_user(
        &self,
        user_id: i64,
        request: &PageRequest,
    ) -> Result<Page<GroupRecord>> {
        self.db.list_groups_for_user(user_id, request).await
    }

    async fn search_groups_for_user(
        &self,
        user_id: i64,
        query: &str,
        limit: u32,
    ) -> Result<Vec<GroupRecord>> {
        self.db.search_groups_for_user(user_id, query, limit).await
    }

    async fn get_group_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Option<GroupRecord>> {
        self.db.get_group_for_user(user_id, group_path).await
    }

    async fn create_group(
        &self,
        actor: &NamespaceActor,
        request: &CreateGroupInput,
    ) -> Result<GroupRecord> {
        self.db.create_group(actor, request).await
    }

    async fn update_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord> {
        self.db.update_group(actor, group_path, request).await
    }

    async fn move_group(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &MoveGroupInput,
    ) -> Result<GroupRecord> {
        self.db.move_group(actor, group_path, request).await
    }

    async fn delete_group(&self, actor: &NamespaceActor, group_path: &str) -> Result<()> {
        self.db.delete_group(actor, group_path).await
    }

    async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &PageRequest,
    ) -> Result<Page<NamespaceMemberRecord>> {
        self.db.list_group_members(actor, group_path, request).await
    }

    async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        self.db
            .upsert_group_member(actor, group_path, request)
            .await
    }

    async fn delete_group_member(
        &self,
        actor: &NamespaceActor,
        group_path: &str,
        login_name: &str,
    ) -> Result<()> {
        self.db
            .delete_group_member(actor, group_path, login_name)
            .await
    }

    async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
        request: &PageRequest,
    ) -> Result<Page<GroupRecord>> {
        self.db
            .list_child_groups_for_user(user_id, group_path, request)
            .await
    }

    async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope> {
        self.db.resolve_access_scope(user_id, group_path).await
    }
}
