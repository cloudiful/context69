use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "runtime_qdrant_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub singleton: bool,
    #[sea_orm(column_type = "Text")]
    pub url: String,
    #[sea_orm(column_type = "Text")]
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
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
            ColumnDef::new(Column::Singleton)
                .boolean()
                .not_null()
                .default(true)
                .primary_key(),
        )
        .col(ColumnDef::new(Column::Url).text().not_null())
        .col(ColumnDef::new(Column::CollectionName).text().not_null())
        .col(
            ColumnDef::new(Column::RecreateOnDimensionMismatch)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "runtime_qdrant_settings_singleton_check",
            Expr::cust("singleton"),
        ))
        .check((
            "runtime_qdrant_settings_url_check",
            Expr::cust("btrim(url) <> ''"),
        ))
        .check((
            "runtime_qdrant_settings_collection_name_check",
            Expr::cust("btrim(collection_name) <> ''"),
        ))
        .to_owned()
}
