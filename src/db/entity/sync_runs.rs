use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, IndexOrder, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "sync_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(column_type = "Text")]
    pub source_key: String,
    #[sea_orm(column_type = "Text")]
    pub trigger_type: String,
    #[sea_orm(column_type = "Text")]
    pub status: String,
    pub records_seen: i32,
    pub records_changed: i32,
    pub chunks_upserted: i32,
    #[sea_orm(column_type = "Text", nullable)]
    pub error_message: Option<String>,
    pub started_at: DateTimeUtc,
    pub finished_at: Option<DateTimeUtc>,
    pub updated_at: DateTimeUtc,
    pub group_id: i64,
    #[sea_orm(column_type = "Text")]
    pub visibility: String,
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
                .custom(Alias::new("bigserial"))
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::SourceKey).text().not_null())
        .col(ColumnDef::new(Column::TriggerType).text().not_null())
        .col(ColumnDef::new(Column::Status).text().not_null())
        .col(
            ColumnDef::new(Column::RecordsSeen)
                .integer()
                .not_null()
                .default(0),
        )
        .col(
            ColumnDef::new(Column::RecordsChanged)
                .integer()
                .not_null()
                .default(0),
        )
        .col(
            ColumnDef::new(Column::ChunksUpserted)
                .integer()
                .not_null()
                .default(0),
        )
        .col(ColumnDef::new(Column::ErrorMessage).text())
        .col(
            ColumnDef::new(Column::StartedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(ColumnDef::new(Column::FinishedAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .check((
            "chk_sync_runs_visibility",
            Expr::cust("visibility IS NULL OR visibility IN ('public', 'private')"),
        ))
        .to_owned()
}
pub fn source_started_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_sync_runs_source_started_at")
        .table((Alias::new("context69"), Entity))
        .col(Column::SourceKey)
        .col((Column::StartedAt, IndexOrder::Desc))
        .to_owned()
}
pub fn group_started_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_sync_runs_group_started_at")
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::SourceKey)
        .col((Column::StartedAt, IndexOrder::Desc))
        .to_owned()
}
