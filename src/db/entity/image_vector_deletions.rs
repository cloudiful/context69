use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

/// One durable image-vector deletion target.
///
/// A delete records the exact physical collection and revision-aware point id
/// that must lose the image vector before the file row disappears. The row is
/// retried by the bounded cleanup pass until the index confirms the removal,
/// and it is never guessed from the current settings.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "image_vector_deletions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub group_id: i64,
    pub file_id: Uuid,
    pub attempt_id: Uuid,
    pub point_id: Uuid,
    #[sea_orm(column_type = "Text")]
    pub content_sha256: String,
    #[sea_orm(column_type = "Text")]
    pub runtime_fingerprint: String,
    #[sea_orm(column_type = "Text")]
    pub endpoint: String,
    #[sea_orm(column_type = "Text")]
    pub collection_name: String,
    pub attempts: i32,
    #[sea_orm(column_type = "Text")]
    pub last_error: Option<String>,
    pub purged_at: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
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
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::FileId).uuid().not_null())
        .col(ColumnDef::new(Column::AttemptId).uuid().not_null())
        .col(ColumnDef::new(Column::PointId).uuid().not_null())
        .col(ColumnDef::new(Column::ContentSha256).text().not_null())
        .col(ColumnDef::new(Column::RuntimeFingerprint).text().not_null())
        .col(ColumnDef::new(Column::Endpoint).text().not_null())
        .col(ColumnDef::new(Column::CollectionName).text().not_null())
        .col(
            ColumnDef::new(Column::Attempts)
                .integer()
                .not_null()
                .default(0),
        )
        .col(ColumnDef::new(Column::LastError).text())
        .col(ColumnDef::new(Column::PurgedAt).timestamp_with_time_zone())
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
            "image_vector_deletions_endpoint_check",
            Expr::cust("btrim(endpoint) <> ''"),
        ))
        .check((
            "image_vector_deletions_collection_check",
            Expr::cust("btrim(collection_name) <> ''"),
        ))
        .to_owned()
}

/// One pending target per physical collection and exact attempt point.
pub fn unique_target_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_image_vector_deletions_target")
        .table((Alias::new("context69"), Entity))
        .col(Column::Endpoint)
        .col(Column::CollectionName)
        .col(Column::PointId)
        .unique()
        .to_owned()
}

/// Pending targets in claim order.
pub fn pending_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_image_vector_deletions_pending")
        .table((Alias::new("context69"), Entity))
        .col(Column::PurgedAt)
        .col(Column::UpdatedAt)
        .to_owned()
}
