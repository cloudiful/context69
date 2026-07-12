mod documents;
mod members;
mod source_folders;

use context69_contracts::{
    CreateGroupRequest, GroupResponse, MoveGroupRequest, UpdateGroupRequest,
};
use reqwest::Method;

use super::{Context69Client, GroupLibraryApi};
use crate::{Error, client::transport::group_path};

pub use documents::{GroupDocumentsApi, GroupMetadataIndexesApi};
pub use members::{GroupMemberApi, GroupMembersApi};
pub use source_folders::{GroupSourceFolderApi, GroupSourceFoldersApi};

pub struct GroupsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> GroupsApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn list(&self) -> Result<Vec<GroupResponse>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/groups")
                    .await?,
            )
            .await
    }

    pub async fn create(&self, request: &CreateGroupRequest) -> Result<GroupResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/groups")
                    .await?
                    .json(request),
            )
            .await
    }
}

pub struct GroupApi<'a> {
    client: &'a Context69Client,
    group_path: String,
}

impl<'a> GroupApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, group_path: String) -> Self {
        Self { client, group_path }
    }

    pub async fn get(&self) -> Result<GroupResponse, Error> {
        let path = group_path(&self.group_path, "");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn update(&self, request: &UpdateGroupRequest) -> Result<GroupResponse, Error> {
        let path = group_path(&self.group_path, "");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PATCH, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn move_to(&self, request: &MoveGroupRequest) -> Result<GroupResponse, Error> {
        let path = group_path(&self.group_path, "/move");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete(&self) -> Result<(), Error> {
        let path = group_path(&self.group_path, "");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub fn children(&self) -> GroupChildrenApi<'a> {
        GroupChildrenApi::new(self.client, self.group_path.clone())
    }

    pub fn members(&self) -> GroupMembersApi<'a> {
        GroupMembersApi::new(self.client, self.group_path.clone())
    }

    pub fn member(&self, login_name: impl Into<String>) -> GroupMemberApi<'a> {
        GroupMemberApi::new(self.client, self.group_path.clone(), login_name.into())
    }

    pub fn library(&self) -> GroupLibraryApi<'a> {
        GroupLibraryApi::new(self.client, self.group_path.clone())
    }

    pub fn source_folders(&self) -> GroupSourceFoldersApi<'a> {
        GroupSourceFoldersApi::new(self.client, self.group_path.clone())
    }

    pub fn documents(&self) -> GroupDocumentsApi<'a> {
        GroupDocumentsApi::new(self.client, self.group_path.clone())
    }

    pub fn metadata_indexes(&self, source_key: impl Into<String>) -> GroupMetadataIndexesApi<'a> {
        GroupMetadataIndexesApi::new(self.client, self.group_path.clone(), source_key.into())
    }

    pub fn source_folder(&self, folder_id: uuid::Uuid) -> GroupSourceFolderApi<'a> {
        GroupSourceFolderApi::new(self.client, self.group_path.clone(), folder_id)
    }
}

pub struct GroupChildrenApi<'a> {
    client: &'a Context69Client,
    group_path: String,
}

impl<'a> GroupChildrenApi<'a> {
    fn new(client: &'a Context69Client, group_path: String) -> Self {
        Self { client, group_path }
    }

    pub async fn list(&self) -> Result<Vec<GroupResponse>, Error> {
        let path = group_path(&self.group_path, "/children");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }
}
