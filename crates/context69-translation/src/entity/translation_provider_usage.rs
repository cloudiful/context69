use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Index, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "translation_provider_usage")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub provider_key: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub usage_month: Date,
    pub character_count: i64,
    pub updated_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::ProviderKey).text().not_null())
        .col(ColumnDef::new(Column::UsageMonth).date().not_null())
        .col(
            ColumnDef::new(Column::CharacterCount)
                .big_integer()
                .not_null()
                .default(0),
        )
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .primary_key(
            Index::create()
                .col(Column::ProviderKey)
                .col(Column::UsageMonth),
        )
        .check((
            "chk_translation_usage_nonnegative",
            Expr::cust("character_count >= 0"),
        ))
        .to_owned()
}
