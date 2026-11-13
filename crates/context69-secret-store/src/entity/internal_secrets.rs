use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};
use sea_orm::sea_query::{ConditionalStatement, ExprTrait};

#[derive(Clone, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "internal_secrets")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub key: String,
    pub value: Vec<u8>,
    pub created_at: DateTimeUtc,
    #[sea_orm(column_type = "Text", nullable)]
    pub purpose: Option<String>,
    pub key_version: i32,
    pub ciphertext_version: i32,
    pub updated_at: Option<DateTimeUtc>,
}
impl std::fmt::Debug for Model {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Model")
            .field("key", &self.key)
            .field("value", &"<redacted>")
            .finish_non_exhaustive()
    }
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create().table((Alias::new("context69"), Entity)).if_not_exists()
        .col(ColumnDef::new(Column::Key).text().not_null().primary_key())
        .col(ColumnDef::new(Column::Value).binary().not_null())
        .col(ColumnDef::new(Column::CreatedAt).timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
        .col(ColumnDef::new(Column::Purpose).text())
        .col(ColumnDef::new(Column::KeyVersion).integer().not_null().default(0))
        .col(ColumnDef::new(Column::CiphertextVersion).integer().not_null().default(0))
        .col(ColumnDef::new(Column::UpdatedAt).timestamp_with_time_zone())
        .check(("chk_internal_secrets_ciphertext_metadata", Expr::cust("key_version >= 0 AND ciphertext_version >= 0 AND ((ciphertext_version = 0) = (purpose IS NULL))")))
        .to_owned()
}

pub fn unversioned_index() -> IndexCreateStatement {
    Index::create()
        .name("internal_secrets_unversioned_idx")
        .table((Alias::new("context69"), Entity))
        .col(Column::Key)
        .and_where(Expr::col(Column::CiphertextVersion).eq(0))
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn declaration_preserves_secret_storage_contract() {
        let table = schema().to_string(PostgresQueryBuilder);
        assert!(table.contains("\"value\" bytea NOT NULL"), "{table}");
        assert!(
            table.contains("\"updated_at\" timestamp with time zone"),
            "{table}"
        );
        assert!(
            table.contains("(ciphertext_version = 0) = (purpose IS NULL)"),
            "{table}"
        );
        let index = unversioned_index().to_string(PostgresQueryBuilder);
        assert!(
            index.contains("WHERE \"ciphertext_version\" = 0"),
            "{index}"
        );
    }
}
