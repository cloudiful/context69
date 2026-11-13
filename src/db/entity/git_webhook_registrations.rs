use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "git_webhook_registrations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub repository_key: Uuid,
    pub provider_kind: String,
    pub external_hook_id: String,
    pub ownership: String,
    pub active: bool,
    pub signing_secret_key: Option<String>,
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
        .col(
            ColumnDef::new(Column::RepositoryKey)
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Column::ProviderKind).text().not_null())
        .col(ColumnDef::new(Column::ExternalHookId).text().not_null())
        .col(ColumnDef::new(Column::Ownership).text().not_null())
        .col(
            ColumnDef::new(Column::Active)
                .boolean()
                .not_null()
                .default(true),
        )
        .col(ColumnDef::new(Column::SigningSecretKey).text())
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
            "chk_git_webhook_registrations_hook",
            Expr::cust("btrim(external_hook_id) <> ''"),
        ))
        .check((
            "chk_git_webhook_registrations_ownership",
            Expr::cust("ownership IN ('integration', 'external', 'unknown')"),
        ))
        .to_owned()
}
pub fn hook_unique_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_git_webhook_registrations_hook")
        .unique()
        .table((Alias::new("context69"), Entity))
        .col(Column::ProviderKind)
        .col(Column::ExternalHookId)
        .to_owned()
}
