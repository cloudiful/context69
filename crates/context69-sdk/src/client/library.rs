use context69_contracts::{
    CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse, LibraryFolderResponse,
    LibraryIngestJobResponse, LibraryTreeResponse, LibraryUploadResponse, MoveFileRequest,
    MoveFolderRequest, UpsertLibraryTextRequest,
};
use reqwest::{Method, multipart::Part};
use uuid::Uuid;

use super::file_upload_form;
use crate::{Context69Client, Error, client::encode_path_component};

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

    pub async fn get_group_library_tree(
        &self,
        group_path: &str,
    ) -> Result<LibraryTreeResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/tree",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn create_group_library_folder(
        &self,
        group_path: &str,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/folders",
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

    pub async fn create_group_library_text(
        &self,
        group_path: &str,
        request: &CreateTextRequest,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/texts",
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

    pub async fn upsert_group_library_text(
        &self,
        group_path: &str,
        request: &UpsertLibraryTextRequest,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/texts",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn move_group_library_folder(
        &self,
        group_path: &str,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/folders/{folder_id}/move",
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

    pub async fn delete_group_library_folder(
        &self,
        group_path: &str,
        folder_id: Uuid,
    ) -> Result<(), Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/folders/{folder_id}",
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

    pub async fn upload_group_library_files(
        &self,
        group_path: &str,
        folder_id: Option<Uuid>,
        files: Vec<Part>,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/files/upload",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .multipart(file_upload_form(folder_id, files)),
            )
            .await
    }

    pub async fn get_group_library_file(
        &self,
        group_path: &str,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/files/{file_id}",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn move_group_library_file(
        &self,
        group_path: &str,
        file_id: Uuid,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/files/{file_id}/move",
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

    pub async fn delete_group_library_file(
        &self,
        group_path: &str,
        file_id: Uuid,
    ) -> Result<(), Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/files/{file_id}",
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

    pub async fn get_group_library_job(
        &self,
        group_path: &str,
        job_id: Uuid,
    ) -> Result<LibraryIngestJobResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/library/jobs/{job_id}",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }
}
