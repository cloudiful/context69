use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "group_memberships")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub group_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: i64,
    #[sea_orm(column_type = "Text")]
    pub role: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::UserId).big_integer().not_null())
        .col(ColumnDef::new(Column::Role).text().not_null())
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
        .primary_key(Index::create().col(Column::GroupId).col(Column::UserId))
        .check((
            "group_memberships_role_check",
            Expr::cust("role IN ('owner', 'maintainer', 'viewer')"),
        ))
        .to_owned()
}
pub fn user_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_group_memberships_user_id")
        .table((Alias::new("context69"), Entity))
        .col(Column::UserId)
        .col(Column::GroupId)
        .to_owned()
}
