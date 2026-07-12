use context69_contracts::{
    GroupTranslationSettingsResponse, RebuildDocumentTranslationsRequest, TranslationJobResponse,
    TranslationJobsResponse, UpdateGroupTranslationSettingsRequest,
};
use reqwest::Method;
use uuid::Uuid;

use super::super::Context69Client;
use crate::{Error, client::transport::group_path};

pub struct GroupTranslationApi<'a> {
    client: &'a Context69Client,
    group_path: String,
}

impl<'a> GroupTranslationApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, group_path: String) -> Self {
        Self { client, group_path }
    }

    pub async fn settings(&self) -> Result<GroupTranslationSettingsResponse, Error> {
        let path = group_path(&self.group_path, "/translation-settings");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn update_settings(
        &self,
        request: &UpdateGroupTranslationSettingsRequest,
    ) -> Result<GroupTranslationSettingsResponse, Error> {
        let path = group_path(&self.group_path, "/translation-settings");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn jobs(&self, document_id: i64) -> Result<TranslationJobsResponse, Error> {
        let path = group_path(
            &self.group_path,
            &format!("/documents/{document_id}/translations"),
        );
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn rebuild(
        &self,
        document_id: i64,
        request: &RebuildDocumentTranslationsRequest,
    ) -> Result<TranslationJobsResponse, Error> {
        let path = group_path(
            &self.group_path,
            &format!("/documents/{document_id}/translations/rebuild"),
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
}

pub struct TranslationJobApi<'a> {
    client: &'a Context69Client,
    group_path: String,
    job_id: Uuid,
}

impl<'a> TranslationJobApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, group_path: String, job_id: Uuid) -> Self {
        Self {
            client,
            group_path,
            job_id,
        }
    }

    pub async fn get(&self) -> Result<TranslationJobResponse, Error> {
        self.request(Method::GET, "").await
    }

    pub async fn retry(&self) -> Result<TranslationJobResponse, Error> {
        self.request(Method::POST, "/retry").await
    }

    async fn request(&self, method: Method, suffix: &str) -> Result<TranslationJobResponse, Error> {
        let path = group_path(
            &self.group_path,
            &format!("/translation-jobs/{}{suffix}", self.job_id),
        );
        self.client
            .execute_json(self.client.authorized_request(method, &path).await?)
            .await
    }
}
