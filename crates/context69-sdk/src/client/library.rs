use context69_contracts::{
    CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse, LibraryFolderResponse,
    LibraryIngestJobResponse, LibraryTreeResponse, LibraryUploadResponse, MoveFileRequest,
    MoveFolderRequest, UpsertLibraryTextRequest,
};
use reqwest::{Method, multipart::Part};
use uuid::Uuid;

use super::file_upload_form;
use crate::{Context69Client, Error};

pub struct LibraryApi<'a> {
    client: &'a Context69Client,
}

impl<'a> LibraryApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn get_library_tree(&self) -> Result<LibraryTreeResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/library/tree")
                    .await?,
            )
            .await
    }

    pub async fn create_library_folder(
        &self,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/library/folders")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn create_library_text(
        &self,
        request: &CreateTextRequest,
    ) -> Result<LibraryUploadResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/library/texts")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn move_library_folder(
        &self,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        let path = format!("/v1/library/folders/{folder_id}/move");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_library_folder(&self, folder_id: Uuid) -> Result<(), Error> {
        let path = format!("/v1/library/folders/{folder_id}");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn upload_library_files(
        &self,
        folder_id: Option<Uuid>,
        files: Vec<Part>,
    ) -> Result<LibraryUploadResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/library/files/upload")
                    .await?
                    .multipart(file_upload_form(folder_id, files)),
            )
            .await
    }

    pub async fn get_library_file(
        &self,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path = format!("/v1/library/files/{file_id}");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn move_library_file(
        &self,
        file_id: Uuid,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path = format!("/v1/library/files/{file_id}/move");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_library_file(&self, file_id: Uuid) -> Result<(), Error> {
        let path = format!("/v1/library/files/{file_id}");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn get_library_job(&self, job_id: Uuid) -> Result<LibraryIngestJobResponse, Error> {
        let path = format!("/v1/library/jobs/{job_id}");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn get_project_library_tree(
        &self,
        group_key: &str,
        project_key: &str,
    ) -> Result<LibraryTreeResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/tree");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn create_project_library_folder(
        &self,
        group_key: &str,
        project_key: &str,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/folders");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn create_project_library_text(
        &self,
        group_key: &str,
        project_key: &str,
        request: &CreateTextRequest,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/texts");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn upsert_project_library_text(
        &self,
        group_key: &str,
        project_key: &str,
        request: &UpsertLibraryTextRequest,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/texts");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn move_project_library_folder(
        &self,
        group_key: &str,
        project_key: &str,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        let path = format!(
            "/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}/move"
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

    pub async fn delete_project_library_folder(
        &self,
        group_key: &str,
        project_key: &str,
        folder_id: Uuid,
    ) -> Result<(), Error> {
        let path =
            format!("/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn upload_project_library_files(
        &self,
        group_key: &str,
        project_key: &str,
        folder_id: Option<Uuid>,
        files: Vec<Part>,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/files/upload");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .multipart(file_upload_form(folder_id, files)),
            )
            .await
    }

    pub async fn get_project_library_file(
        &self,
        group_key: &str,
        project_key: &str,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn move_project_library_file(
        &self,
        group_key: &str,
        project_key: &str,
        file_id: Uuid,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path =
            format!("/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}/move");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_project_library_file(
        &self,
        group_key: &str,
        project_key: &str,
        file_id: Uuid,
    ) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn get_project_library_job(
        &self,
        group_key: &str,
        project_key: &str,
        job_id: Uuid,
    ) -> Result<LibraryIngestJobResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/jobs/{job_id}");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }
}
