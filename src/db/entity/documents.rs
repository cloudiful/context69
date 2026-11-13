use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, IndexOrder, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "documents")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTimeUtc>,
    pub updated_at_source: DateTimeUtc,
    pub metadata_json: Json,
    pub record_hash: String,
    pub last_synced_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub group_id: i64,
    pub visibility: String,
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
                .big_integer()
                .not_null()
                .auto_increment()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::SourceKey).text().not_null())
        .col(ColumnDef::new(Column::ExternalId).text().not_null())
        .col(ColumnDef::new(Column::Title).text().not_null())
        .col(ColumnDef::new(Column::Summary).text())
        .col(ColumnDef::new(Column::SourceUri).text().not_null())
        .col(ColumnDef::new(Column::PublishedAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::UpdatedAtSource)
                .timestamp_with_time_zone()
                .not_null(),
        )
        .col(
            ColumnDef::new(Column::MetadataJson)
                .json_binary()
                .not_null()
                .default(Expr::cust("'{}'::jsonb")),
        )
        .col(ColumnDef::new(Column::RecordHash).text().not_null())
        .col(
            ColumnDef::new(Column::LastSyncedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
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
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .check((
            "chk_documents_visibility",
            Expr::cust("visibility IS NULL OR visibility IN ('public', 'private')"),
        ))
        .to_owned()
}
pub fn group_source_external_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_documents_group_source_external_id")
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::SourceKey)
        .col(Column::ExternalId)
        .to_owned()
}
pub fn source_published_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_documents_source_published_at")
        .table((Alias::new("context69"), Entity))
        .col(Column::SourceKey)
        .col((Column::PublishedAt, IndexOrder::Desc))
        .to_owned()
}
/// Declares the index installed only when the pg_trgm extension exists.
pub fn optional_pg_trgm_index() -> IndexCreateStatement {
    use sea_orm::sea_query::{IndexType, IntoIden};
    Index::create()
        .name("documents_title_trgm_idx")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .index_type(IndexType::Custom(Alias::new("gin").into_iden()))
        .col(Expr::cust("lower(title) gin_trgm_ops"))
        .to_owned()
}
pub fn group_source_published_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_documents_group_source_published_id")
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::SourceKey)
        .col((Column::PublishedAt, IndexOrder::Desc))
        .col((Column::Id, IndexOrder::Asc))
        .to_owned()
}
pub fn library_file_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_documents_library_file_id")
        .table((Alias::new("context69"), Entity))
        .col(Expr::cust("(metadata_json ->> 'library_file_id')"))
        .to_owned()
}
pub fn record_hash_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_documents_record_hash")
        .table((Alias::new("context69"), Entity))
        .col(Column::RecordHash)
        .to_owned()
}
pub fn group_published_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_documents_group_published_at")
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col((Column::PublishedAt, IndexOrder::Desc))
        .col((Column::Id, IndexOrder::Desc))
        .to_owned()
}
pub fn visibility_group_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_documents_visibility_group_id")
        .table((Alias::new("context69"), Entity))
        .col(Column::Visibility)
        .col(Column::GroupId)
        .col((Column::Id, IndexOrder::Desc))
        .to_owned()
}
