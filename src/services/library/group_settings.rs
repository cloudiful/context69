use anyhow::Result;
use context69_contracts::{GroupLibrarySettingsResponse, UpdateGroupLibrarySettingsRequest};

use super::LibraryService;

impl LibraryService {
    pub async fn group_library_settings(
        &self,
        group_id: i64,
    ) -> Result<GroupLibrarySettingsResponse> {
        let default_source_policy = self.store.group_library_settings(group_id).await?;
        Ok(GroupLibrarySettingsResponse {
            default_source_policy,
        })
    }

    pub async fn update_group_library_settings(
        &self,
        group_id: i64,
        request: &UpdateGroupLibrarySettingsRequest,
    ) -> Result<GroupLibrarySettingsResponse> {
        let default_source_policy = self
            .store
            .update_group_library_settings(group_id, request.default_source_policy)
            .await?;
        Ok(GroupLibrarySettingsResponse {
            default_source_policy,
        })
    }
}
