use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, ConditionalStatement, Expr, Index, IndexCreateStatement, Table,
    TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "library_storage_objects")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub group_id: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_backend: String,
    pub object_key: String,
    pub staging_lease_until: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

/// The declaration of the storage object table.
///
/// The foreign key to `context69.groups` stays migration-owned: this
/// declaration describes the columns, the primary key and the checks of the
/// applied schema and is never executed.
pub fn schema() -> TableCreateStatement {
    let mut group_sha256_unique = Index::create();
    group_sha256_unique
        .name("library_storage_objects_group_id_sha256_key")
        .unique()
        .col(Column::GroupId)
        .col(Column::Sha256);
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Sha256).text().not_null())
        .col(ColumnDef::new(Column::SizeBytes).big_integer().not_null())
        .col(ColumnDef::new(Column::StorageBackend).text().not_null())
        .col(ColumnDef::new(Column::ObjectKey).text().not_null())
        .col(ColumnDef::new(Column::StagingLeaseUntil).timestamp_with_time_zone())
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
            "chk_library_storage_objects_sha256",
            Expr::cust("length(sha256) = 64"),
        ))
        .check((
            "chk_library_storage_objects_size_bytes",
            Expr::cust("size_bytes >= 0"),
        ))
        .check((
            "chk_library_storage_objects_storage_backend",
            Expr::cust("storage_backend IN ('local', 's3')"),
        ))
        .check((
            "chk_library_storage_objects_object_key",
            Expr::cust("btrim(object_key) <> ''"),
        ))
        .index(&mut group_sha256_unique)
        .to_owned()
}

/// The scan of the staged objects whose lease expired.
pub fn staging_lease_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_storage_objects_staging_lease")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::StagingLeaseUntil)
        .cond_where(Expr::cust("staging_lease_until IS NOT NULL"))
        .to_owned()
}
