mod files;
mod folders;
mod texts;

use context69_contracts::{LibraryIngestJobResponse, LibraryTreeResponse};
use reqwest::Method;
use uuid::Uuid;

use super::Context69Client;
use crate::{Error, client::transport::group_path};

pub use files::{LibraryFileApi, LibraryFilesApi};
pub use folders::{LibraryFolderApi, LibraryFoldersApi};
pub use texts::{GroupLibraryTextsApi, LibraryTextsApi};

pub struct LibraryApi<'a> {
    client: &'a Context69Client,
}

impl<'a> LibraryApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn tree(&self) -> Result<LibraryTreeResponse, Error> {
        get_tree(self.client, "/v1/library").await
    }

    pub fn folders(&self) -> LibraryFoldersApi<'a> {
        LibraryFoldersApi::new(self.client, "/v1/library".to_string())
    }

    pub fn folder(&self, folder_id: Uuid) -> LibraryFolderApi<'a> {
        LibraryFolderApi::new(self.client, "/v1/library".to_string(), folder_id)
    }

    pub fn texts(&self) -> LibraryTextsApi<'a> {
        LibraryTextsApi::new(self.client, "/v1/library".to_string())
    }

    pub fn files(&self) -> LibraryFilesApi<'a> {
        LibraryFilesApi::new(self.client, "/v1/library".to_string())
    }

    pub fn file(&self, file_id: Uuid) -> LibraryFileApi<'a> {
        LibraryFileApi::new(self.client, "/v1/library".to_string(), file_id)
    }

    pub fn job(&self, job_id: Uuid) -> LibraryJobApi<'a> {
        LibraryJobApi::new(self.client, "/v1/library".to_string(), job_id)
    }
}

pub struct GroupLibraryApi<'a> {
    client: &'a Context69Client,
    base_path: String,
}

impl<'a> GroupLibraryApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, group: String) -> Self {
        Self {
            client,
            base_path: group_path(&group, "/library"),
        }
    }

    pub async fn tree(&self) -> Result<LibraryTreeResponse, Error> {
        get_tree(self.client, &self.base_path).await
    }

    pub fn folders(&self) -> LibraryFoldersApi<'a> {
        LibraryFoldersApi::new(self.client, self.base_path.clone())
    }

    pub fn folder(&self, folder_id: Uuid) -> LibraryFolderApi<'a> {
        LibraryFolderApi::new(self.client, self.base_path.clone(), folder_id)
    }

    pub fn texts(&self) -> GroupLibraryTextsApi<'a> {
        GroupLibraryTextsApi::new(self.client, self.base_path.clone())
    }

    pub fn files(&self) -> LibraryFilesApi<'a> {
        LibraryFilesApi::new(self.client, self.base_path.clone())
    }

    pub fn file(&self, file_id: Uuid) -> LibraryFileApi<'a> {
        LibraryFileApi::new(self.client, self.base_path.clone(), file_id)
    }

    pub fn job(&self, job_id: Uuid) -> LibraryJobApi<'a> {
        LibraryJobApi::new(self.client, self.base_path.clone(), job_id)
    }
}

pub struct LibraryJobApi<'a> {
    client: &'a Context69Client,
    base_path: String,
    job_id: Uuid,
}

impl<'a> LibraryJobApi<'a> {
    fn new(client: &'a Context69Client, base_path: String, job_id: Uuid) -> Self {
        Self {
            client,
            base_path,
            job_id,
        }
    }

    pub async fn get(&self) -> Result<LibraryIngestJobResponse, Error> {
        let path = format!("{}/jobs/{}", self.base_path, self.job_id);
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }
}

async fn get_tree(client: &Context69Client, base_path: &str) -> Result<LibraryTreeResponse, Error> {
    let path = format!("{base_path}/tree");
    client
        .execute_json(client.authorized_request(Method::GET, &path).await?)
        .await
}
