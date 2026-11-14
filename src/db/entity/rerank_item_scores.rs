use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "rerank_item_scores")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub rerank_model: String,
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub query_hash: String,
    #[sea_orm(column_type = "Text")]
    pub query_text_trimmed: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub chunk_id: Uuid,
    pub score: f32,
    pub created_at: DateTimeWithTimeZone,
    pub last_used_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::RerankModel).text().not_null())
        .col(ColumnDef::new(Column::QueryHash).text().not_null())
        .col(ColumnDef::new(Column::QueryTextTrimmed).text().not_null())
        .col(ColumnDef::new(Column::ChunkId).uuid().not_null())
        .col(ColumnDef::new(Column::Score).float().not_null())
        .col(
            ColumnDef::new(Column::CreatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(
            ColumnDef::new(Column::LastUsedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .primary_key(
            Index::create()
                .col(Column::RerankModel)
                .col(Column::QueryHash)
                .col(Column::ChunkId),
        )
        .to_owned()
}

pub fn last_used_index() -> IndexCreateStatement {
    Index::create()
        .name("rerank_item_scores_last_used_at_idx")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::LastUsedAt)
        .to_owned()
}
