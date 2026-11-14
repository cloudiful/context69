use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "search_generations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub scope: String,
    pub generation: i64,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(
            ColumnDef::new(Column::Scope)
                .text()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::Generation).big_integer().not_null())
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .to_owned()
}
