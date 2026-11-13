use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "groups")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub parent_group_id: Option<i64>,
    #[sea_orm(column_type = "Text")]
    pub group_key: String,
    #[sea_orm(column_type = "Text")]
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub visibility: String,
    #[sea_orm(column_type = "Text")]
    pub kind: String,
    pub owner_user_id: Option<i64>,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(column_type = "Text")]
    pub full_path: String,
    #[sea_orm(column_type = "Text")]
    pub default_source_policy: String,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    let mut full_path_unique = Index::create();
    full_path_unique
        .name("uq_groups_full_path")
        .unique()
        .col(Column::FullPath);
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(
            ColumnDef::new(Column::Id)
                .custom(Alias::new("bigserial"))
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::ParentGroupId).big_integer())
        .col(ColumnDef::new(Column::GroupKey).text().not_null())
        .col(ColumnDef::new(Column::Name).text().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .col(ColumnDef::new(Column::Kind).text().not_null())
        .col(ColumnDef::new(Column::OwnerUserId).big_integer())
        .col(ColumnDef::new(Column::CreatedByUserId).big_integer())
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
        .col(ColumnDef::new(Column::FullPath).text().not_null())
        .col(
            ColumnDef::new(Column::DefaultSourcePolicy)
                .text()
                .not_null()
                .default("retain"),
        )
        .check((
            "groups_group_key_check",
            Expr::cust("btrim(group_key) <> ''"),
        ))
        .check(("groups_name_check", Expr::cust("btrim(name) <> ''")))
        .check((
            "groups_visibility_check",
            Expr::cust("visibility IN ('public', 'private')"),
        ))
        .check((
            "groups_kind_check",
            Expr::cust("kind IN ('personal', 'shared')"),
        ))
        .check((
            "groups_default_source_policy_check",
            Expr::cust("default_source_policy IN ('retain', 'release_after_processing')"),
        ))
        .index(&mut full_path_unique)
        .to_owned()
}
pub fn parent_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_groups_parent_group_id")
        .table((Alias::new("context69"), Entity))
        .col(Column::ParentGroupId)
        .to_owned()
}
pub fn parent_group_key_unique_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_groups_parent_group_key")
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Expr::cust("COALESCE(parent_group_id, 0)"))
        .col(Column::GroupKey)
        .to_owned()
}
