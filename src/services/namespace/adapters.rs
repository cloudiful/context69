use anyhow::Result;
use context69_namespace::{
    AccessScope, CreateGroupInput, CreateProjectInput, GroupRecord, MoveProjectInput,
    NamespaceActor, NamespaceMemberRecord, NamespaceRepository, ProjectRecord, UpdateGroupInput,
    UpdateProjectInput, UpsertMembershipInput,
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

impl NamespaceRepository for DbNamespaceRepository {
    async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupRecord>> {
        self.db.list_groups_for_user(user_id).await
    }

    async fn get_group_for_user(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Option<GroupRecord>> {
        self.db.get_group_for_user(user_id, group_key).await
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
        group_key: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord> {
        self.db.update_group(actor, group_key, request).await
    }

    async fn delete_group(&self, actor: &NamespaceActor, group_key: &str) -> Result<()> {
        self.db.delete_group(actor, group_key).await
    }

    async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.db.list_group_members(actor, group_key).await
    }

    async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        self.db.upsert_group_member(actor, group_key, request).await
    }

    async fn delete_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        login_name: &str,
    ) -> Result<()> {
        self.db
            .delete_group_member(actor, group_key, login_name)
            .await
    }

    async fn list_projects_for_user_in_group(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Vec<ProjectRecord>> {
        self.db
            .list_projects_for_user_in_group(user_id, group_key)
            .await
    }

    async fn get_project_for_user(
        &self,
        user_id: i64,
        group_key: &str,
        project_key: &str,
    ) -> Result<Option<ProjectRecord>> {
        self.db
            .get_project_for_user(user_id, group_key, project_key)
            .await
    }

    async fn create_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &CreateProjectInput,
    ) -> Result<ProjectRecord> {
        self.db.create_project(actor, group_key, request).await
    }

    async fn update_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectInput,
    ) -> Result<ProjectRecord> {
        self.db
            .update_project(actor, group_key, project_key, request)
            .await
    }

    async fn delete_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<()> {
        self.db.delete_project(actor, group_key, project_key).await
    }

    async fn move_project(
        &self,
        actor: &NamespaceActor,
        source_group_key: &str,
        project_key: &str,
        request: &MoveProjectInput,
    ) -> Result<ProjectRecord> {
        self.db
            .move_project(actor, source_group_key, project_key, request)
            .await
    }

    async fn list_project_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.db
            .list_project_members(actor, group_key, project_key)
            .await
    }

    async fn upsert_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        self.db
            .upsert_project_member(actor, group_key, project_key, request)
            .await
    }

    async fn delete_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<()> {
        self.db
            .delete_project_member(actor, group_key, project_key, login_name)
            .await
    }

    async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<AccessScope> {
        self.db
            .resolve_access_scope(user_id, group_key, project_key)
            .await
    }
}
