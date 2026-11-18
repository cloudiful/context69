use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(
    schema_name = "context69",
    table_name = "runtime_image_embedding_settings"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub singleton: bool,
    pub enabled: bool,
    #[sea_orm(column_type = "Text")]
    pub base_url: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub collection_name: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub model: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub query_model: Option<String>,
    pub dimensions: Option<i64>,
    pub timeout_secs: Option<i64>,
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
        .col(
            ColumnDef::new(Column::Enabled)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(ColumnDef::new(Column::BaseUrl).text())
        .col(ColumnDef::new(Column::CollectionName).text())
        .col(ColumnDef::new(Column::Model).text())
        .col(ColumnDef::new(Column::QueryModel).text())
        .col(ColumnDef::new(Column::Dimensions).big_integer())
        .col(ColumnDef::new(Column::TimeoutSecs).big_integer())
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "runtime_image_embedding_settings_singleton_check",
            Expr::cust("singleton"),
        ))
        .check((
            "runtime_image_embedding_settings_base_url_check",
            Expr::cust("base_url IS NULL OR btrim(base_url) <> ''"),
        ))
        .check((
            "runtime_image_embedding_settings_collection_name_check",
            Expr::cust("collection_name IS NULL OR btrim(collection_name) <> ''"),
        ))
        .check((
            "runtime_image_embedding_settings_model_check",
            Expr::cust("model IS NULL OR btrim(model) <> ''"),
        ))
        .check((
            "runtime_image_embedding_settings_query_model_check",
            Expr::cust("query_model IS NULL OR btrim(query_model) <> ''"),
        ))
        .check((
            "runtime_image_embedding_settings_dimensions_check",
            Expr::cust("dimensions IS NULL OR dimensions > 0"),
        ))
        .check((
            "runtime_image_embedding_settings_timeout_check",
            Expr::cust("timeout_secs IS NULL OR timeout_secs > 0"),
        ))
        .to_owned()
}
