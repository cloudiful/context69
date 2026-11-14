use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, ConditionalStatement, Expr, Index, IndexCreateStatement, Table,
    TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(
    schema_name = "context69",
    table_name = "library_legacy_object_cleanup"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    pub group_id: i64,
    pub file_id: Uuid,
    pub old_key: String,
    pub old_storage_backend: Option<String>,
    pub migrated_at: DateTimeUtc,
    pub cleanup_eligible_at: DateTimeUtc,
    pub deleted_at: Option<DateTimeUtc>,
    pub delete_error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

/// The declaration of the legacy cleanup table.
///
/// The table deliberately carries no foreign keys, so the cleanup data
/// survives the deletion of its source file and group. The declaration
/// describes the columns, the primary key and the checks of the applied schema
/// and is never executed.
pub fn schema() -> TableCreateStatement {
    let mut old_key_unique = Index::create();
    old_key_unique
        .name("library_legacy_object_cleanup_file_id_old_key_key")
        .unique()
        .col(Column::FileId)
        .col(Column::OldKey);
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(
            ColumnDef::new(Column::Id)
                .big_integer()
                .not_null()
                .primary_key()
                .extra("GENERATED ALWAYS AS IDENTITY"),
        )
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::FileId).uuid().not_null())
        .col(ColumnDef::new(Column::OldKey).text().not_null())
        .col(ColumnDef::new(Column::OldStorageBackend).text())
        .col(
            ColumnDef::new(Column::MigratedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(
            ColumnDef::new(Column::CleanupEligibleAt)
                .timestamp_with_time_zone()
                .not_null(),
        )
        .col(ColumnDef::new(Column::DeletedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::DeleteError).text())
        .check((
            "library_legacy_object_cleanup_old_key_check",
            Expr::cust("btrim(old_key) <> ''"),
        ))
        .check((
            "library_legacy_object_cleanup_old_storage_backend_check",
            Expr::cust("old_storage_backend IS NULL OR btrim(old_storage_backend) <> ''"),
        ))
        .check((
            "library_legacy_object_cleanup_deleted_at_check",
            Expr::cust("deleted_at IS NULL OR deleted_at >= migrated_at"),
        ))
        .index(&mut old_key_unique)
        .to_owned()
}

/// The scan of the open cleanup records whose grace period elapsed.
pub fn eligible_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_legacy_object_cleanup_eligible")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::CleanupEligibleAt)
        .cond_where(Expr::cust("deleted_at IS NULL"))
        .to_owned()
}
