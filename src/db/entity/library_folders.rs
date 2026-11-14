use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, IndexOrder, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "library_folders")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub group_id: i64,
    pub visibility: String,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

/// The declaration of the folder table.
///
/// The foreign keys to `context69.groups` and to the parent folder stay
/// migration-owned: this declaration describes the columns, the primary key
/// and the checks of the applied schema and is never executed.
pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .col(ColumnDef::new(Column::ParentId).uuid())
        .col(ColumnDef::new(Column::Name).text().not_null())
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
        .check((
            "chk_library_folders_visibility",
            Expr::cust("visibility IN ('public', 'private')"),
        ))
        .check(("chk_library_folders_name", Expr::cust("btrim(name) <> ''")))
        .to_owned()
}

/// The unique folder name inside one group, with the root folder spelled out.
pub fn group_parent_name_unique_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_library_folders_group_parent_name")
        .if_not_exists()
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Expr::cust("COALESCE(parent_id::text, '__root__')"))
        .col(Column::Name)
        .to_owned()
}

/// The folder lookup index of one group.
pub fn group_parent_name_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_folders_group_parent_name")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::ParentId)
        .col(Column::Name)
        .to_owned()
}

/// The folder pagination index of one group.
pub fn group_parent_updated_at_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_folders_group_parent_updated_at")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::ParentId)
        .col((Column::UpdatedAt, IndexOrder::Desc))
        .col(Column::Id)
        .to_owned()
}
