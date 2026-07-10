use context69_contracts::{
    CreateGroupRequest, GroupMemberResponse, GroupResponse, MoveGroupRequest, UpdateGroupRequest,
    UpsertMembershipRequest, UserDirectoryEntryResponse,
};
use reqwest::Method;

use crate::{Context69Client, Error, client::encode_path_component};

pub struct WorkspaceApi<'a> {
    client: &'a Context69Client,
}

impl<'a> WorkspaceApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn search_user_directory(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UserDirectoryEntryResponse>, Error> {
        let mut url = self.client.url("/v1/user-directory")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("query", query);
            pairs.append_pair("limit", &limit.to_string());
        }
        self.client
            .execute_json(self.client.authorized_url_request(Method::GET, url).await?)
            .await
    }

    pub async fn list_groups(&self) -> Result<Vec<GroupResponse>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/groups")
                    .await?,
            )
            .await
    }

    pub async fn create_group(&self, request: &CreateGroupRequest) -> Result<GroupResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/groups")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn get_group(&self, group_path: &str) -> Result<GroupResponse, Error> {
        let path = format!("/v1/groups/by-path/{}", encode_path_component(group_path));
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn update_group(
        &self,
        group_path: &str,
        request: &UpdateGroupRequest,
    ) -> Result<GroupResponse, Error> {
        let path = format!("/v1/groups/by-path/{}", encode_path_component(group_path));
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PATCH, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn move_group(
        &self,
        group_path: &str,
        request: &MoveGroupRequest,
    ) -> Result<GroupResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/move",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_group(&self, group_path: &str) -> Result<(), Error> {
        let path = format!("/v1/groups/by-path/{}", encode_path_component(group_path));
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn list_child_groups(&self, group_path: &str) -> Result<Vec<GroupResponse>, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/children",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn list_group_members(
        &self,
        group_path: &str,
    ) -> Result<Vec<GroupMemberResponse>, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/members",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn upsert_group_member(
        &self,
        group_path: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<(), Error> {
        let path = format!(
            "/v1/groups/by-path/{}/members",
            encode_path_component(group_path)
        );
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_group_member(
        &self,
        group_path: &str,
        login_name: &str,
    ) -> Result<(), Error> {
        let path = format!(
            "/v1/groups/by-path/{}/members/{login_name}",
            encode_path_component(group_path)
        );
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}
