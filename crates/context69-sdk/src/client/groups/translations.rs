use context69_contracts::{
    GroupTranslationSettingsResponse, RebuildDocumentTranslationsRequest, TaskRef,
    TranslationJobsResponse, UpdateGroupTranslationSettingsRequest,
};
use reqwest::Method;

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
    ) -> Result<TaskRef, Error> {
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
