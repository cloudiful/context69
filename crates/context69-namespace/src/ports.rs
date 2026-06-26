use anyhow::Result;

use crate::{
    AccessScope, CreateGroupInput, CreateProjectInput, GroupRecord, MoveProjectInput,
    NamespaceActor, NamespaceMemberRecord, ProjectRecord, UpdateGroupInput, UpdateProjectInput,
    UpsertMembershipInput,
};

#[allow(async_fn_in_trait)]
pub trait NamespaceRepository: Send + Sync {
    async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupRecord>>;
    async fn get_group_for_user(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Option<GroupRecord>>;
    async fn create_group(
        &self,
        actor: &NamespaceActor,
        request: &CreateGroupInput,
    ) -> Result<GroupRecord>;
    async fn update_group(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &UpdateGroupInput,
    ) -> Result<GroupRecord>;
    async fn delete_group(&self, actor: &NamespaceActor, group_key: &str) -> Result<()>;
    async fn list_group_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>>;
    async fn upsert_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()>;
    async fn delete_group_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        login_name: &str,
    ) -> Result<()>;
    async fn list_projects_for_user_in_group(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Vec<ProjectRecord>>;
    async fn get_project_for_user(
        &self,
        user_id: i64,
        group_key: &str,
        project_key: &str,
    ) -> Result<Option<ProjectRecord>>;
    async fn create_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        request: &CreateProjectInput,
    ) -> Result<ProjectRecord>;
    async fn update_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectInput,
    ) -> Result<ProjectRecord>;
    async fn delete_project(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<()>;
    async fn move_project(
        &self,
        actor: &NamespaceActor,
        source_group_key: &str,
        project_key: &str,
        request: &MoveProjectInput,
    ) -> Result<ProjectRecord>;
    async fn list_project_members(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>>;
    async fn upsert_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipInput,
    ) -> Result<()>;
    async fn delete_project_member(
        &self,
        actor: &NamespaceActor,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<()>;
    async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<AccessScope>;
}
