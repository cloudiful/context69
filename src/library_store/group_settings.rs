use anyhow::Result;
use context69_contracts::SourcePolicy;
use sqlx::FromRow;

use super::LibraryStore;
use crate::domain_errors::DomainError;

#[derive(Debug, Clone, FromRow)]
struct GroupLibrarySettingsRow {
    default_source_policy: String,
}

impl LibraryStore {
    pub async fn group_library_settings(&self, group_id: i64) -> Result<SourcePolicy> {
        let row = sqlx::query_file_as!(
            GroupLibrarySettingsRow,
            "src/sql/library_store/group_settings/get.sql",
            group_id
        )
        .fetch_optional(self.db.pool())
        .await?
        .ok_or_else(|| DomainError::not_found("unknown group"))?;
        source_policy_from_stored(&row.default_source_policy)
    }

    pub async fn update_group_library_settings(
        &self,
        group_id: i64,
        default_source_policy: SourcePolicy,
    ) -> Result<SourcePolicy> {
        let row = sqlx::query_file_as!(
            GroupLibrarySettingsRow,
            "src/sql/library_store/group_settings/update.sql",
            group_id,
            default_source_policy.as_str()
        )
        .fetch_optional(self.db.pool())
        .await?
        .ok_or_else(|| DomainError::not_found("unknown group"))?;
        source_policy_from_stored(&row.default_source_policy)
    }
}

fn source_policy_from_stored(value: &str) -> Result<SourcePolicy> {
    value
        .parse()
        .map_err(|_| DomainError::internal("stored group default source policy is invalid").into())
}
