use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(column_type = "Text")]
    pub login_name: String,
    #[sea_orm(column_type = "Text")]
    pub display_name: String,
    #[sea_orm(column_type = "Text")]
    pub password_hash: String,
    pub is_admin: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub disabled_at: Option<DateTimeUtc>,
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
        .col(
            ColumnDef::new(Column::LoginName)
                .text()
                .not_null()
                .unique_key(),
        )
        .col(ColumnDef::new(Column::DisplayName).text().not_null())
        .col(ColumnDef::new(Column::PasswordHash).text().not_null())
        .col(
            ColumnDef::new(Column::IsAdmin)
                .boolean()
                .not_null()
                .default(false),
        )
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
        .col(ColumnDef::new(Column::DisabledAt).timestamp_with_time_zone())
        .check((
            "users_login_name_check",
            Expr::cust("btrim(login_name) <> ''"),
        ))
        .check((
            "users_display_name_check",
            Expr::cust("btrim(display_name) <> ''"),
        ))
        .check((
            "users_password_hash_check",
            Expr::cust("btrim(password_hash) <> ''"),
        ))
        .to_owned()
}
