use anyhow::Result;
use context69_namespace::{
    CreateGroupInput, CreateProjectInput, MoveProjectInput, NamespaceActor,
    NamespaceService as CoreNamespaceService, UpdateGroupInput, UpdateProjectInput,
    UpsertMembershipInput,
};

use crate::{
    contracts::{
        CreateGroupRequest, CreateProjectRequest, MoveProjectRequest, UpdateGroupRequest,
        UpdateProjectRequest, UpsertMembershipRequest,
    },
    db::Database,
    domain::{AccessScope, GroupRecord, NamespaceMemberRecord, ProjectRecord, UserRecord},
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
        group_key: &str,
    ) -> Result<Option<GroupRecord>> {
        self.inner.get_group_for_user(user_id, group_key).await
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
        group_key: &str,
        request: &UpdateGroupRequest,
    ) -> Result<GroupRecord> {
        self.inner
            .update_group(
                &namespace_actor(actor),
                group_key,
                &update_group_input(request),
            )
            .await
    }

    pub async fn delete_group(&self, actor: &UserRecord, group_key: &str) -> Result<()> {
        self.inner
            .delete_group(&namespace_actor(actor), group_key)
            .await
    }

    pub async fn list_group_members(
        &self,
        actor: &UserRecord,
        group_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.inner
            .list_group_members(&namespace_actor(actor), group_key)
            .await
    }

    pub async fn upsert_group_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()> {
        self.inner
            .upsert_group_member(
                &namespace_actor(actor),
                group_key,
                &upsert_membership_input(request),
            )
            .await
    }

    pub async fn delete_group_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        login_name: &str,
    ) -> Result<()> {
        self.inner
            .delete_group_member(&namespace_actor(actor), group_key, login_name)
            .await
    }

    pub async fn list_projects_for_user_in_group(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Vec<ProjectRecord>> {
        self.inner
            .list_projects_for_user_in_group(user_id, group_key)
            .await
    }

    pub async fn get_project_for_user(
        &self,
        user_id: i64,
        group_key: &str,
        project_key: &str,
    ) -> Result<Option<ProjectRecord>> {
        self.inner
            .get_project_for_user(user_id, group_key, project_key)
            .await
    }

    pub async fn create_project(
        &self,
        actor: &UserRecord,
        group_key: &str,
        request: &CreateProjectRequest,
    ) -> Result<ProjectRecord> {
        self.inner
            .create_project(
                &namespace_actor(actor),
                group_key,
                &create_project_input(request),
            )
            .await
    }

    pub async fn update_project(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectRequest,
    ) -> Result<ProjectRecord> {
        self.inner
            .update_project(
                &namespace_actor(actor),
                group_key,
                project_key,
                &update_project_input(request),
            )
            .await
    }

    pub async fn delete_project(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
    ) -> Result<()> {
        self.inner
            .delete_project(&namespace_actor(actor), group_key, project_key)
            .await
    }

    pub async fn move_project(
        &self,
        actor: &UserRecord,
        source_group_key: &str,
        project_key: &str,
        request: &MoveProjectRequest,
    ) -> Result<ProjectRecord> {
        self.inner
            .move_project(
                &namespace_actor(actor),
                source_group_key,
                project_key,
                &move_project_input(request),
            )
            .await
    }

    pub async fn list_project_members(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<NamespaceMemberRecord>> {
        self.inner
            .list_project_members(&namespace_actor(actor), group_key, project_key)
            .await
    }

    pub async fn upsert_project_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()> {
        self.inner
            .upsert_project_member(
                &namespace_actor(actor),
                group_key,
                project_key,
                &upsert_membership_input(request),
            )
            .await
    }

    pub async fn delete_project_member(
        &self,
        actor: &UserRecord,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<()> {
        self.inner
            .delete_project_member(&namespace_actor(actor), group_key, project_key, login_name)
            .await
    }

    pub async fn resolve_access_scope(
        &self,
        user_id: Option<i64>,
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<AccessScope> {
        self.inner
            .resolve_access_scope(user_id, group_key, project_key)
            .await
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
        parent_group_key: request.parent_group_key.clone(),
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

fn create_project_input(request: &CreateProjectRequest) -> CreateProjectInput {
    CreateProjectInput {
        project_key: request.project_key.clone(),
        name: request.name.clone(),
        visibility: request.visibility,
    }
}

fn update_project_input(request: &UpdateProjectRequest) -> UpdateProjectInput {
    UpdateProjectInput {
        name: request.name.clone(),
        visibility: request.visibility,
    }
}

fn move_project_input(request: &MoveProjectRequest) -> MoveProjectInput {
    MoveProjectInput {
        target_group_key: request.target_group_key.clone(),
    }
}

fn upsert_membership_input(request: &UpsertMembershipRequest) -> UpsertMembershipInput {
    UpsertMembershipInput {
        login_name: request.login_name.clone(),
        role: request.role,
    }
}
