use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Index, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "runtime_source_connections")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub name: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub connection_key: Uuid,
    #[sea_orm(column_type = "Text", nullable)]
    pub database_url_secret_key: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    let mut connection_key_unique = Index::create();
    connection_key_unique
        .name("uq_runtime_source_connections_connection_key")
        .unique()
        .col(Column::ConnectionKey);
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::Name).text().not_null().primary_key())
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
        .col(
            ColumnDef::new(Column::ConnectionKey)
                .uuid()
                .not_null()
                .default(Expr::cust("gen_random_uuid()")),
        )
        .col(ColumnDef::new(Column::DatabaseUrlSecretKey).text())
        .check((
            "runtime_source_connections_name_check",
            Expr::cust("btrim(name) <> ''"),
        ))
        .index(&mut connection_key_unique)
        .to_owned()
}
