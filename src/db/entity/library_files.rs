use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, ColumnType, ConditionalStatement, Expr, Index, IndexCreateStatement,
    IndexOrder, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "library_files")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub group_id: i64,
    pub visibility: String,
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub source_uri: Option<String>,
    pub published_at: Option<DateTimeUtc>,
    pub metadata_json: serde_json::Value,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
    pub delete_source_after_processing: bool,
    pub source_released_at: Option<DateTimeUtc>,
    pub ingest_status: String,
    pub error_message: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub ingested_at: Option<DateTimeUtc>,
    pub translation_override: bool,
    pub translation_source_locale: Option<String>,
    pub translation_target_locales: Vec<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

/// The declaration of the library file table.
///
/// The foreign keys to `context69.groups`, to the parent folder and to the
/// storage object stay migration-owned: this declaration describes the
/// columns, the primary key and the checks of the applied schema and is never
/// executed.
pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::Visibility).text().not_null())
        .col(ColumnDef::new(Column::FolderId).uuid())
        .col(ColumnDef::new(Column::ExternalId).text())
        .col(ColumnDef::new(Column::SourceUri).text())
        .col(ColumnDef::new(Column::PublishedAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::MetadataJson)
                .json_binary()
                .not_null()
                .default(Expr::cust("'{}'::jsonb")),
        )
        .col(ColumnDef::new(Column::Filename).text().not_null())
        .col(ColumnDef::new(Column::MediaType).text().not_null())
        .col(ColumnDef::new(Column::SizeBytes).big_integer().not_null())
        .col(ColumnDef::new(Column::Sha256).text().not_null())
        .col(ColumnDef::new(Column::StorageRelPath).text().not_null())
        .col(ColumnDef::new(Column::StorageObjectId).uuid())
        .col(
            ColumnDef::new(Column::DeleteSourceAfterProcessing)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(ColumnDef::new(Column::SourceReleasedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::IngestStatus).text().not_null())
        .col(ColumnDef::new(Column::ErrorMessage).text())
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
        .col(ColumnDef::new(Column::IngestedAt).timestamp_with_time_zone())
        .col(
            ColumnDef::new(Column::TranslationOverride)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(ColumnDef::new(Column::TranslationSourceLocale).text())
        .col(
            ColumnDef::new(Column::TranslationTargetLocales)
                .array(ColumnType::Text)
                .not_null()
                .default(Expr::cust("ARRAY[]::TEXT[]")),
        )
        .check((
            "chk_library_files_visibility",
            Expr::cust("visibility IN ('public', 'private')"),
        ))
        .check((
            "chk_library_files_size_bytes",
            Expr::cust("size_bytes >= 0"),
        ))
        .check((
            "chk_library_files_filename",
            Expr::cust("btrim(filename) <> ''"),
        ))
        .check((
            "chk_library_files_storage_rel_path",
            Expr::cust("btrim(storage_rel_path) <> ''"),
        ))
        .to_owned()
}

/// The unique file name inside one group folder, with the group root spelled
/// out.
pub fn group_folder_filename_unique_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_library_files_group_folder_filename")
        .if_not_exists()
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Expr::cust("COALESCE(folder_id::text, '__root__')"))
        .col(Column::Filename)
        .to_owned()
}

/// The unique external identity of one file inside one group.
pub fn group_external_id_unique_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_library_files_group_external_id")
        .if_not_exists()
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::ExternalId)
        .cond_where(Expr::cust("external_id IS NOT NULL"))
        .to_owned()
}

/// The file lookup index of one group folder.
pub fn group_folder_filename_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_files_group_folder_filename")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::FolderId)
        .col(Column::Filename)
        .to_owned()
}

/// The file pagination index of one group folder.
pub fn group_folder_updated_at_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_files_group_folder_updated_at")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::FolderId)
        .col((Column::UpdatedAt, IndexOrder::Desc))
        .col(Column::Id)
        .to_owned()
}

/// The file size pagination index of one group folder.
pub fn group_folder_size_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_files_group_folder_size")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::FolderId)
        .col((Column::SizeBytes, IndexOrder::Desc))
        .col(Column::Id)
        .to_owned()
}

/// The file status pagination index of one group folder.
pub fn group_folder_status_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_files_group_folder_status")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::FolderId)
        .col(Column::IngestStatus)
        .col(Column::Id)
        .to_owned()
}

/// The reverse lookup of a file by its storage object.
pub fn storage_object_id_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_files_storage_object_id")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::StorageObjectId)
        .cond_where(Expr::cust("storage_object_id IS NOT NULL"))
        .to_owned()
}

/// The auto-release scan of the files whose source is still owed.
pub fn pending_source_release_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_files_pending_source_release")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::Id)
        .cond_where(Expr::cust(
            "delete_source_after_processing AND source_released_at IS NULL",
        ))
        .to_owned()
}

/// The dedup lookup of one content hash inside one group.
pub fn group_sha256_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_library_files_group_sha256")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::Sha256)
        .col(Column::CreatedAt)
        .col(Column::Id)
        .to_owned()
}
