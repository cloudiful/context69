use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(
    schema_name = "context69",
    table_name = "document_translation_attempts"
)]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub job_id: Uuid,
    pub provider_key: String,
    pub provider_config_hash: String,
    pub attempt_number: i32,
    pub status: String,
    pub character_count: i64,
    pub latency_ms: i64,
    pub error_message: Option<String>,
    pub created_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(
            ColumnDef::new(Column::Id)
                .big_integer()
                .not_null()
                .auto_increment()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::JobId).uuid().not_null())
        .col(ColumnDef::new(Column::ProviderKey).text().not_null())
        .col(ColumnDef::new(Column::ProviderConfigHash).text().not_null())
        .col(ColumnDef::new(Column::AttemptNumber).integer().not_null())
        .col(ColumnDef::new(Column::Status).text().not_null())
        .col(
            ColumnDef::new(Column::CharacterCount)
                .big_integer()
                .not_null(),
        )
        .col(ColumnDef::new(Column::LatencyMs).big_integer().not_null())
        .col(ColumnDef::new(Column::ErrorMessage).text())
        .col(
            ColumnDef::new(Column::CreatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "chk_translation_attempt_status",
            Expr::cust("status IN ('succeeded', 'failed', 'quota_exceeded')"),
        ))
        .to_owned()
}
