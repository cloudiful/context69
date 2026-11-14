use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, IndexOrder, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "personal_access_tokens")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: i64,
    #[sea_orm(column_type = "Text")]
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub token_hash: String,
    #[sea_orm(column_type = "Text")]
    pub display_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: DateTimeUtc,
    pub last_used_at: Option<DateTimeUtc>,
    pub revoked_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Column::UserId).big_integer().not_null())
        .col(ColumnDef::new(Column::Name).text().not_null())
        .col(
            ColumnDef::new(Column::TokenHash)
                .text()
                .not_null()
                .unique_key(),
        )
        .col(ColumnDef::new(Column::DisplayPrefix).text().not_null())
        .col(
            ColumnDef::new(Column::Scopes)
                .array(ColumnType::Text)
                .not_null(),
        )
        .col(
            ColumnDef::new(Column::ExpiresAt)
                .timestamp_with_time_zone()
                .not_null(),
        )
        .col(ColumnDef::new(Column::LastUsedAt).timestamp_with_time_zone())
        .col(ColumnDef::new(Column::RevokedAt).timestamp_with_time_zone())
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
            "personal_access_tokens_name_check",
            Expr::cust("btrim(name) <> ''"),
        ))
        .check((
            "personal_access_tokens_token_hash_check",
            Expr::cust("btrim(token_hash) <> ''"),
        ))
        .check((
            "personal_access_tokens_display_prefix_check",
            Expr::cust("btrim(display_prefix) <> ''"),
        ))
        .check((
            "personal_access_tokens_scopes_check",
            Expr::cust("cardinality(scopes) > 0"),
        ))
        .to_owned()
}

pub fn user_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_personal_access_tokens_user_id")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::UserId)
        .col((Column::CreatedAt, IndexOrder::Desc))
        .to_owned()
}

pub fn active_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_personal_access_tokens_active")
        .if_not_exists()
        .table((Alias::new("context69"), Entity))
        .col(Column::UserId)
        .col(Column::RevokedAt)
        .col(Column::ExpiresAt)
        .to_owned()
}
