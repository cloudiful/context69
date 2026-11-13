use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "document_chunks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub document_id: i64,
    pub chunk_index: i32,
    #[sea_orm(column_type = "Text")]
    pub chunk_text: String,
    #[sea_orm(column_type = "Text")]
    pub record_hash: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    let mut chunk_unique = Index::create();
    chunk_unique
        .name("document_chunks_document_id_chunk_index_key")
        .unique()
        .col(Column::DocumentId)
        .col(Column::ChunkIndex);
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Column::DocumentId).big_integer().not_null())
        .col(ColumnDef::new(Column::ChunkIndex).integer().not_null())
        .col(ColumnDef::new(Column::ChunkText).text().not_null())
        .col(ColumnDef::new(Column::RecordHash).text().not_null())
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
        .index(&mut chunk_unique)
        .to_owned()
}

/// Declares the index installed only when the pg_trgm extension exists.
pub fn optional_pg_trgm_index() -> IndexCreateStatement {
    use sea_orm::sea_query::{IndexType, IntoIden};
    Index::create()
        .name("document_chunks_text_trgm_idx")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .index_type(IndexType::Custom(Alias::new("gin").into_iden()))
        .col(Expr::cust("lower(chunk_text) gin_trgm_ops"))
        .to_owned()
}

pub fn record_hash_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_document_chunks_record_hash")
        .table((Alias::new("context69"), Entity))
        .col(Column::RecordHash)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn optional_trigram_index_preserves_operator_class() {
        let sql = optional_pg_trgm_index().to_string(PostgresQueryBuilder);
        assert!(sql.contains("lower(chunk_text) gin_trgm_ops"), "{sql}");
        assert!(
            sql.contains("USING gin") || sql.contains("USING \"gin\""),
            "{sql}"
        );
    }
}
