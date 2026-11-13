use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "source_configs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub source_key: String,
    #[sea_orm(column_type = "Text")]
    pub connection: String,
    #[sea_orm(column_type = "Text")]
    pub sync_strategy: String,
    #[sea_orm(column_type = "Text")]
    pub connector_type: String,
    #[sea_orm(column_type = "Text")]
    pub base_query: String,
    pub batch_size: i64,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(column_type = "Text", nullable)]
    pub display_name: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub description: Option<String>,
    pub example_queries: Json,
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
            ColumnDef::new(Column::SourceKey)
                .text()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::Connection).text().not_null())
        .col(ColumnDef::new(Column::SyncStrategy).text().not_null())
        .col(ColumnDef::new(Column::ConnectorType).text().not_null())
        .col(ColumnDef::new(Column::BaseQuery).text().not_null())
        .col(ColumnDef::new(Column::BatchSize).big_integer().not_null())
        .col(
            ColumnDef::new(Column::CreatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(ColumnDef::new(Column::DisplayName).text())
        .col(ColumnDef::new(Column::Description).text())
        .col(
            ColumnDef::new(Column::ExampleQueries)
                .json_binary()
                .not_null()
                .default(Expr::cust("'[]'::jsonb")),
        )
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .check((
            "source_configs_batch_size_check",
            Expr::cust("batch_size > 0"),
        ))
        .check((
            "chk_source_configs_visibility",
            Expr::cust("visibility IS NULL OR visibility IN ('public', 'private')"),
        ))
        .to_owned()
}
pub fn connection_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_source_configs_connection")
        .table((Alias::new("context69"), Entity))
        .col(Column::Connection)
        .to_owned()
}
pub fn group_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_source_configs_group_id")
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::SourceKey)
        .to_owned()
}
