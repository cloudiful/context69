use context69_contracts::{GroupMemberPageResponse, NamespacePageQuery, UpsertMembershipRequest};
use reqwest::Method;

use super::Context69Client;
use crate::{
    Error,
    client::transport::{encode_path_component, group_path},
};

pub struct GroupMembersApi<'a> {
    client: &'a Context69Client,
    group_path: String,
}

impl<'a> GroupMembersApi<'a> {
    pub(super) fn new(client: &'a Context69Client, group_path: String) -> Self {
        Self { client, group_path }
    }

    pub async fn list(&self) -> Result<GroupMemberPageResponse, Error> {
        let path = group_path(&self.group_path, "/members");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn list_page(
        &self,
        query: &NamespacePageQuery,
    ) -> Result<GroupMemberPageResponse, Error> {
        let path = group_path(&self.group_path, "/members");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, &path)
                    .await?
                    .query(query),
            )
            .await
    }

    pub async fn upsert(&self, request: &UpsertMembershipRequest) -> Result<(), Error> {
        let path = group_path(&self.group_path, "/members");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }
}

pub struct GroupMemberApi<'a> {
    client: &'a Context69Client,
    group_path: String,
    login_name: String,
}

impl<'a> GroupMemberApi<'a> {
    pub(super) fn new(client: &'a Context69Client, group_path: String, login_name: String) -> Self {
        Self {
            client,
            group_path,
            login_name,
        }
    }

    pub async fn delete(&self) -> Result<(), Error> {
        let suffix = format!("/members/{}", encode_path_component(&self.login_name));
        let path = group_path(&self.group_path, &suffix);
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}
