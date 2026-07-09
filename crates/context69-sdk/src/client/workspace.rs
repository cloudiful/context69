use context69_contracts::{
    CreateGroupRequest, CreateProjectRequest, GroupMemberResponse, GroupResponse,
    MoveProjectRequest, ProjectMemberResponse, ProjectResponse, UpdateGroupRequest,
    UpdateProjectRequest, UpsertMembershipRequest, UserDirectoryEntryResponse,
};
use reqwest::Method;

use crate::{Context69Client, Error};

impl Context69Client {
    pub async fn search_user_directory(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UserDirectoryEntryResponse>, Error> {
        let mut url = self.url("/v1/user-directory")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("query", query);
            pairs.append_pair("limit", &limit.to_string());
        }
        self.execute_json(self.client.get(url).header(
            reqwest::header::AUTHORIZATION,
            format!(
                        "Bearer {}",
                        self.session
                            .read()
                            .await
                            .personal_access_token
                            .clone()
                            .ok_or(Error::AuthenticationRequired)?
                    ),
        ))
        .await
    }

    pub async fn list_groups(&self) -> Result<Vec<GroupResponse>, Error> {
        self.execute_json(self.authorized_request(Method::GET, "/v1/groups").await?)
            .await
    }

    pub async fn create_group(&self, request: &CreateGroupRequest) -> Result<GroupResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/groups")
                .await?
                .json(request),
        )
        .await
    }

    pub async fn get_group(&self, group_key: &str) -> Result<GroupResponse, Error> {
        let path = format!("/v1/groups/{group_key}");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn update_group(
        &self,
        group_key: &str,
        request: &UpdateGroupRequest,
    ) -> Result<GroupResponse, Error> {
        let path = format!("/v1/groups/{group_key}");
        self.execute_json(
            self.authorized_request(Method::PATCH, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn delete_group(&self, group_key: &str) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }

    pub async fn list_group_members(
        &self,
        group_key: &str,
    ) -> Result<Vec<GroupMemberResponse>, Error> {
        let path = format!("/v1/groups/{group_key}/members");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn upsert_group_member(
        &self,
        group_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}/members");
        self.execute_empty(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn delete_group_member(
        &self,
        group_key: &str,
        login_name: &str,
    ) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}/members/{login_name}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }

    pub async fn list_projects(&self, group_key: &str) -> Result<Vec<ProjectResponse>, Error> {
        let path = format!("/v1/groups/{group_key}/projects");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn create_project(
        &self,
        group_key: &str,
        request: &CreateProjectRequest,
    ) -> Result<ProjectResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects");
        self.execute_json(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn get_project(
        &self,
        group_key: &str,
        project_key: &str,
    ) -> Result<ProjectResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn update_project(
        &self,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectRequest,
    ) -> Result<ProjectResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}");
        self.execute_json(
            self.authorized_request(Method::PATCH, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn delete_project(&self, group_key: &str, project_key: &str) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }

    pub async fn move_project(
        &self,
        group_key: &str,
        project_key: &str,
        request: &MoveProjectRequest,
    ) -> Result<ProjectResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/move");
        self.execute_json(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn list_project_members(
        &self,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<ProjectMemberResponse>, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/members");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn upsert_project_member(
        &self,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/members");
        self.execute_empty(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn delete_project_member(
        &self,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/members/{login_name}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }
}
