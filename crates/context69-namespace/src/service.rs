use anyhow::Result;

use crate::{
    AccessScope, CreateGroupInput, CreateProjectInput, GroupRecord, MoveProjectInput,
    NamespaceActor, NamespaceMemberRecord, NamespaceRepository, ProjectRecord, UpdateGroupInput,
    UpdateProjectInput, UpsertMembershipInput,
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
        group_key: &str,
    ) -> Result<Option<GroupRecord>> {
        self.repository.get_group_for_user(user_id, group_key).await
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
        group_key: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord> {
        self.repository
            .update_group(actor, group_key, request)
            .await
    }

    pub async fn delete_group(&self, actor: &NamespaceActor, group_key: &str) -> Result<()> {
        self.repository.delete_group(actor, group_key).await
    }

    pub async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.repository.list_group_members(actor, group_key).await
    }

    pub async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        self.repository
            .upsert_group_member(actor, group_key, request)
            .await
    }

    pub async fn delete_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        login_name: &str,
    ) -> Result<()> {
        self.repository
            .delete_group_member(actor, group_key, login_name)
            .await
    }

    pub async fn list_projects_for_user_in_group(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Vec<ProjectRecord>> {
        self.repository
            .list_projects_for_user_in_group(user_id, group_key)
            .await
    }

    pub async fn get_project_for_user(
        &self,
        user_id: i64,
        group_key: &str,
        project_key: &str,
    ) -> Result<Option<ProjectRecord>> {
        self.repository
            .get_project_for_user(user_id, group_key, project_key)
            .await
    }

    pub async fn create_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &CreateProjectInput,
    ) -> Result<ProjectRecord> {
        self.repository
            .create_project(actor, group_key, request)
            .await
    }

    pub async fn update_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectInput,
    ) -> Result<ProjectRecord> {
        self.repository
            .update_project(actor, group_key, project_key, request)
            .await
    }

    pub async fn delete_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<()> {
        self.repository
            .delete_project(actor, group_key, project_key)
            .await
    }

    pub async fn move_project(
        &self,
        actor: &NamespaceActor,
        source_group_key: &str,
        project_key: &str,
        request: &MoveProjectInput,
    ) -> Result<ProjectRecord> {
        self.repository
            .move_project(actor, source_group_key, project_key, request)
            .await
    }

    pub async fn list_project_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.repository
            .list_project_members(actor, group_key, project_key)
            .await
    }

    pub async fn upsert_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()> {
        self.repository
            .upsert_project_member(actor, group_key, project_key, request)
            .await
    }

    pub async fn delete_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<()> {
        self.repository
            .delete_project_member(actor, group_key, project_key, login_name)
            .await
    }

    pub async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<AccessScope> {
        self.repository
            .resolve_access_scope(user_id, group_key, project_key)
            .await
    }
}
