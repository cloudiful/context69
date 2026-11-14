use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "source_checkpoints")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub source_key: String,
    pub cursor_updated_at: Option<DateTimeUtc>,
    #[sea_orm(column_type = "Text", nullable)]
    pub cursor_external_id: Option<String>,
    pub last_success_at: Option<DateTimeUtc>,
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
            ColumnDef::new(Column::SourceKey)
                .text()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::CursorUpdatedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::CursorExternalId).text())
        .col(ColumnDef::new(Column::LastSuccessAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .check((
            "chk_source_checkpoints_visibility",
            Expr::cust("visibility IS NULL OR visibility IN ('public', 'private')"),
        ))
        .to_owned()
}
pub fn group_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_source_checkpoints_group_id")
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::SourceKey)
        .to_owned()
}
