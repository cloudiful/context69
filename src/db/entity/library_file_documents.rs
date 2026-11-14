use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "library_file_documents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub file_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub document_id: i64,
    pub group_id: i64,
    pub visibility: String,
    pub section_key: String,
    pub section_label: String,
    pub section_external_id: Option<String>,
    pub section_source_uri: Option<String>,
    pub section_published_at: Option<DateTimeUtc>,
    pub section_metadata_json: serde_json::Value,
    pub sort_order: i32,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

/// The declaration of the file/document link table.
///
/// The foreign keys to the library file and to the document stay
/// migration-owned: this declaration describes the columns, the composite
/// primary key and the checks of the applied schema and is never executed.
pub fn schema() -> TableCreateStatement {
    let mut primary_key = Index::create();
    primary_key.col(Column::FileId).col(Column::DocumentId);
    let mut section_key_unique = Index::create();
    section_key_unique
        .name("library_file_documents_file_id_section_key_key")
        .unique()
        .col(Column::FileId)
        .col(Column::SectionKey);
    let mut table = Table::create();
    table
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::FileId).uuid().not_null())
        .col(ColumnDef::new(Column::DocumentId).big_integer().not_null())
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .col(ColumnDef::new(Column::SectionKey).text().not_null())
        .col(ColumnDef::new(Column::SectionLabel).text().not_null())
        .col(ColumnDef::new(Column::SectionExternalId).text())
        .col(ColumnDef::new(Column::SectionSourceUri).text())
        .col(ColumnDef::new(Column::SectionPublishedAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::SectionMetadataJson)
                .json_binary()
                .not_null()
                .default(Expr::cust("'{}'::jsonb")),
        )
        .col(
            ColumnDef::new(Column::SortOrder)
                .integer()
                .not_null()
                .default(0),
        )
        .col(
            ColumnDef::new(Column::CreatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "chk_library_file_documents_visibility",
            Expr::cust("visibility IN ('public', 'private')"),
        ));
    table.primary_key(&mut primary_key);
    table.index(&mut section_key_unique);
    table.to_owned()
}

/// The reverse lookup of a link by its document.
pub fn document_id_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_file_documents_document_id")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::DocumentId)
        .to_owned()
}

/// The group lookup of a link by its document.
pub fn group_id_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_file_documents_group_id")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::DocumentId)
        .to_owned()
}
