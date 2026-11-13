use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, ColumnType, ConditionalStatement, Expr, Index, IndexCreateStatement,
    IndexOrder, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "git_repository_sources")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub repository_key: Uuid,
    pub connection_key: Option<String>,
    pub provider_kind: String,
    pub canonical_url: String,
    pub repository_owner: String,
    pub repository_name: String,
    pub default_branch: String,
    pub target_ref: String,
    pub target_commit_sha: Option<String>,
    pub indexed_commit_sha: Option<String>,
    pub index_profile: String,
    pub index_status: String,
    pub last_indexed_at: Option<DateTimeUtc>,
    pub checkpoint_updated_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub group_id: i64,
    pub generation_counter: i64,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub interval_seconds: Option<i64>,
    pub webhook_enabled: bool,
    pub enabled: bool,
    pub retention_count: i64,
    pub last_skip_summary: Json,
    pub indexed_pipeline_version: i64,
    pub indexed_config_fingerprint: String,
    pub created_by_user_id: Option<i64>,
    pub refresh_dirty: bool,
    pub refresh_requested_at: Option<DateTimeUtc>,
    pub last_refresh_attempt_at: Option<DateTimeUtc>,
    pub deletion_requested_at: Option<DateTimeUtc>,
    pub last_error_code: Option<String>,
    pub pinned_commit_sha: Option<String>,
    pub config_revision: i64,
    pub force_rebuild_requested_at: Option<DateTimeUtc>,
    pub deletion_attempts: i32,
    pub deletion_attempted_at: Option<DateTimeUtc>,
    pub deletion_next_retry_at: Option<DateTimeUtc>,
    pub cleanup_deferred_until: Option<DateTimeUtc>,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    let mut group_ref_unique = Index::create();
    group_ref_unique
        .name("uq_git_repository_sources_group_ref")
        .unique()
        .col(Column::GroupId)
        .col(Column::CanonicalUrl)
        .col(Column::TargetRef);
    Table::create().table((Alias::new("context69"), Entity)).if_not_exists()
        .col(ColumnDef::new(Column::RepositoryKey).uuid().not_null().primary_key().default(Expr::cust("gen_random_uuid()")))
        .col(ColumnDef::new(Column::ConnectionKey).text())
        .col(ColumnDef::new(Column::ProviderKind).text().not_null())
        .col(ColumnDef::new(Column::CanonicalUrl).text().not_null())
        .col(ColumnDef::new(Column::RepositoryOwner).text().not_null())
        .col(ColumnDef::new(Column::RepositoryName).text().not_null())
        .col(ColumnDef::new(Column::DefaultBranch).text().not_null())
        .col(ColumnDef::new(Column::TargetRef).text().not_null())
        .col(ColumnDef::new(Column::TargetCommitSha).text())
        .col(ColumnDef::new(Column::IndexedCommitSha).text())
        .col(ColumnDef::new(Column::IndexProfile).text().not_null())
        .col(ColumnDef::new(Column::IndexStatus).text().not_null().default("pending"))
        .col(ColumnDef::new(Column::LastIndexedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::CheckpointUpdatedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::CreatedAt).timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
        .col(ColumnDef::new(Column::UpdatedAt).timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::GenerationCounter).big_integer().not_null().default(0))
        .col(ColumnDef::new(Column::IncludePatterns).array(ColumnType::Text).not_null().default(Expr::cust("'{}'::text[]")))
        .col(ColumnDef::new(Column::ExcludePatterns).array(ColumnType::Text).not_null().default(Expr::cust("'{}'::text[]")))
        .col(ColumnDef::new(Column::IntervalSeconds).big_integer())
        .col(ColumnDef::new(Column::WebhookEnabled).boolean().not_null().default(false))
        .col(ColumnDef::new(Column::Enabled).boolean().not_null().default(true))
        .col(ColumnDef::new(Column::RetentionCount).big_integer().not_null().default(2))
        .col(ColumnDef::new(Column::LastSkipSummary).json_binary().not_null().default(Expr::cust("'{}'::jsonb")))
        .col(ColumnDef::new(Column::IndexedPipelineVersion).big_integer().not_null().default(0))
        .col(ColumnDef::new(Column::IndexedConfigFingerprint).text().not_null().default(""))
        .col(ColumnDef::new(Column::CreatedByUserId).big_integer())
        .col(ColumnDef::new(Column::RefreshDirty).boolean().not_null().default(false))
        .col(ColumnDef::new(Column::RefreshRequestedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::LastRefreshAttemptAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::DeletionRequestedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::LastErrorCode).text())
        .col(ColumnDef::new(Column::PinnedCommitSha).text())
        .col(ColumnDef::new(Column::ConfigRevision).big_integer().not_null().default(0))
        .col(ColumnDef::new(Column::ForceRebuildRequestedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::DeletionAttempts).integer().not_null().default(0))
        .col(ColumnDef::new(Column::DeletionAttemptedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::DeletionNextRetryAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::CleanupDeferredUntil).timestamp_with_time_zone())
        .check(("chk_git_repository_sources_provider", Expr::cust("provider_kind IN ('github', 'forgejo', 'gitlab', 'generic')")))
        .check(("chk_git_repository_sources_url", Expr::cust("btrim(canonical_url) <> ''")))
        .check(("chk_git_repository_sources_owner", Expr::cust("btrim(repository_owner) <> ''")))
        .check(("chk_git_repository_sources_name", Expr::cust("btrim(repository_name) <> ''")))
        .check(("chk_git_repository_sources_branch", Expr::cust("btrim(default_branch) <> ''")))
        .check(("chk_git_repository_sources_ref", Expr::cust("btrim(target_ref) <> ''")))
        .check(("chk_git_repository_sources_profile", Expr::cust("index_profile IN ('lexical', 'hybrid', 'full_semantic')")))
        .check(("chk_git_repository_sources_status", Expr::cust("index_status IN ('pending', 'indexing', 'ready', 'stale', 'failed', 'disabled')")))
        .check(("chk_git_repository_sources_generation_counter", Expr::cust("generation_counter >= 0")))
        .check(("chk_git_repository_sources_interval", Expr::cust("interval_seconds IS NULL OR interval_seconds BETWEEN 300 AND 2592000")))
        .check(("chk_git_repository_sources_retention", Expr::cust("retention_count BETWEEN 1 AND 50")))
        .check(("chk_git_repository_sources_patterns", Expr::cust("cardinality(include_patterns) <= 128 AND cardinality(exclude_patterns) <= 128")))
        .check(("chk_git_repository_sources_skip_summary", Expr::cust("jsonb_typeof(last_skip_summary) = 'object'")))
        .check(("chk_git_repository_sources_pipeline", Expr::cust("indexed_pipeline_version >= 0")))
        .check(("git_repository_sources_last_error_code_check", Expr::cust("last_error_code IS NULL OR (char_length(last_error_code) BETWEEN 1 AND 64 AND last_error_code ~ '^[a-z0-9_]+$')")))
        .index(&mut group_ref_unique)
        .to_owned()
}
pub fn connection_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_repository_sources_connection")
        .table((Alias::new("context69"), Entity))
        .col(Column::ConnectionKey)
        .to_owned()
}
pub fn status_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_repository_sources_status")
        .table((Alias::new("context69"), Entity))
        .col(Column::IndexStatus)
        .col((Column::UpdatedAt, IndexOrder::Desc))
        .to_owned()
}
pub fn group_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_repository_sources_group")
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col((Column::CreatedAt, IndexOrder::Desc))
        .col(Column::RepositoryKey)
        .to_owned()
}
pub fn enabled_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_repository_sources_enabled")
        .table((Alias::new("context69"), Entity))
        .col(Column::Enabled)
        .col(Column::IndexStatus)
        .col((Column::UpdatedAt, IndexOrder::Desc))
        .to_owned()
}
pub fn refresh_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_repository_sources_refresh")
        .table((Alias::new("context69"), Entity))
        .col(Column::Enabled)
        .col(Column::RefreshDirty)
        .col(Column::IntervalSeconds)
        .col(Column::LastRefreshAttemptAt)
        .to_owned()
}
pub fn deletion_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_repository_sources_deletion")
        .table((Alias::new("context69"), Entity))
        .col(Column::DeletionRequestedAt)
        .and_where(Expr::cust("deletion_requested_at IS NOT NULL"))
        .to_owned()
}
pub fn deletion_retry_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_repository_sources_deletion_retry")
        .table((Alias::new("context69"), Entity))
        .col(Column::DeletionNextRetryAt)
        .col(Column::RepositoryKey)
        .and_where(Expr::cust("deletion_requested_at IS NOT NULL"))
        .to_owned()
}
