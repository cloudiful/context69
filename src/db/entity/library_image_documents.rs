use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "library_image_documents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub file_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub document_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub role: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub ordinal: i32,
    pub group_id: i64,
    pub caption: Option<String>,
    pub context_text: Option<String>,
    pub source_uri: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

/// The declaration of the image/document occurrence link table.
///
/// The table is additive and introduces no foreign key: the file, document and
/// group cleanup stays an explicit service responsibility, so this declaration
/// only describes the columns, the composite primary key and the checks of the
/// applied schema and is never executed.
pub fn schema() -> TableCreateStatement {
    let mut primary_key = Index::create();
    primary_key
        .col(Column::FileId)
        .col(Column::DocumentId)
        .col(Column::Role)
        .col(Column::Ordinal);
    let mut table = Table::create();
    table
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::FileId).uuid().not_null())
        .col(ColumnDef::new(Column::DocumentId).big_integer().not_null())
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Role).text().not_null())
        .col(
            ColumnDef::new(Column::Ordinal)
                .integer()
                .not_null()
                .default(0),
        )
        .col(ColumnDef::new(Column::Caption).text())
        .col(ColumnDef::new(Column::ContextText).text())
        .col(ColumnDef::new(Column::SourceUri).text())
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
            "chk_library_image_documents_role",
            Expr::cust("role IN ('cover', 'body')"),
        ))
        .check((
            "chk_library_image_documents_ordinal",
            Expr::cust("ordinal >= 0"),
        ))
        .check((
            "chk_library_image_documents_caption",
            Expr::cust("caption IS NULL OR char_length(caption) <= 1024"),
        ))
        .check((
            "chk_library_image_documents_context",
            Expr::cust("context_text IS NULL OR char_length(context_text) <= 8192"),
        ));
    table.primary_key(&mut primary_key);
    table.to_owned()
}

/// One occurrence slot of a document holds one image, whichever file stores it.
pub fn occurrence_unique_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_library_image_documents_occurrence")
        .if_not_exists()
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Column::DocumentId)
        .col(Column::Role)
        .col(Column::Ordinal)
        .to_owned()
}

/// The scoped lookup of a document's image occurrences inside one group.
pub fn group_document_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_image_documents_group_document")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::DocumentId)
        .to_owned()
}

/// The scoped lookup and cleanup of one file's image occurrences.
pub fn file_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_image_documents_file")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::FileId)
        .to_owned()
}
