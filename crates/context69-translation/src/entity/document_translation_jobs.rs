use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, ConditionalStatement, Expr, Index, IndexCreateStatement, Table,
    TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "document_translation_jobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub document_id: i64,
    pub target_locale: String,
    pub requested_source_locale: Option<String>,
    pub detected_source_locale: Option<String>,
    pub source_record_hash: String,
    pub status: String,
    pub provider_key: Option<String>,
    pub provider_config_hash: Option<String>,
    pub attempt_count: i32,
    pub source_character_count: i64,
    pub error_message: Option<String>,
    pub created_at: DateTimeUtc,
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    pub updated_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create().table((Alias::new("context69"), Entity)).if_not_exists()
        .col(ColumnDef::new(Column::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Column::DocumentId).big_integer().not_null())
        .col(ColumnDef::new(Column::TargetLocale).text().not_null())
        .col(ColumnDef::new(Column::RequestedSourceLocale).text())
        .col(ColumnDef::new(Column::DetectedSourceLocale).text())
        .col(ColumnDef::new(Column::SourceRecordHash).text().not_null())
        .col(ColumnDef::new(Column::Status).text().not_null().default("queued"))
        .col(ColumnDef::new(Column::ProviderKey).text())
        .col(ColumnDef::new(Column::ProviderConfigHash).text())
        .col(ColumnDef::new(Column::AttemptCount).integer().not_null().default(0))
        .col(ColumnDef::new(Column::SourceCharacterCount).big_integer().not_null().default(0))
        .col(ColumnDef::new(Column::ErrorMessage).text())
        .col(ColumnDef::new(Column::CreatedAt).timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
        .col(ColumnDef::new(Column::StartedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::FinishedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::UpdatedAt).timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
        .check(("chk_document_translation_job_status", Expr::cust("status IN ('queued', 'running', 'succeeded', 'failed', 'skipped', 'quota_exceeded')"))).to_owned()
}
pub fn active_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_document_translation_jobs_active")
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Column::DocumentId)
        .col(Column::TargetLocale)
        .col(Column::SourceRecordHash)
        .and_where(Expr::cust("status IN ('queued', 'running')"))
        .to_owned()
}
pub fn pending_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_document_translation_jobs_pending")
        .table((Alias::new("context69"), Entity))
        .col(Column::Status)
        .col(Column::CreatedAt)
        .col(Column::Id)
        .and_where(Expr::cust("status IN ('queued', 'running')"))
        .to_owned()
}
