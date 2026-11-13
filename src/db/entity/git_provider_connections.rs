use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, Index, IndexCreateStatement, Table, TableCreateStatement,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "git_provider_connections")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub group_id: i64,
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub connection_key: String,
    pub provider_kind: String,
    pub connection_mode: String,
    pub display_name: String,
    pub base_url: String,
    pub credential_secret_key: Option<String>,
    pub webhook_secret_key: Option<String>,
    pub app_private_key_secret_key: Option<String>,
    pub disabled_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::groups::Entity",
        from = "Column::GroupId",
        to = "super::groups::Column::Id"
    )]
    Groups,
}
impl Related<super::groups::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Groups.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create()
        .table((Alias::new("context69"), Entity))
        .if_not_exists()
        .col(ColumnDef::new(Column::GroupId).big_integer().not_null())
        .col(ColumnDef::new(Column::ConnectionKey).text().not_null())
        .col(ColumnDef::new(Column::ProviderKind).text().not_null())
        .col(ColumnDef::new(Column::ConnectionMode).text().not_null())
        .col(ColumnDef::new(Column::DisplayName).text().not_null())
        .col(ColumnDef::new(Column::BaseUrl).text().not_null())
        .col(ColumnDef::new(Column::CredentialSecretKey).text())
        .col(ColumnDef::new(Column::WebhookSecretKey).text())
        .col(ColumnDef::new(Column::AppPrivateKeySecretKey).text())
        .col(ColumnDef::new(Column::DisabledAt).timestamp_with_time_zone())
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
        .primary_key(
            Index::create()
                .col(Column::GroupId)
                .col(Column::ConnectionKey),
        )
        .check((
            "chk_git_provider_connections_key",
            Expr::cust("btrim(connection_key) <> ''"),
        ))
        .check((
            "chk_git_provider_connections_provider",
            Expr::cust("provider_kind IN ('github', 'forgejo', 'gitlab', 'generic')"),
        ))
        .check((
            "chk_git_provider_connections_mode",
            Expr::cust("connection_mode IN ('public', 'installation', 'token')"),
        ))
        .check((
            "chk_git_provider_connections_display",
            Expr::cust("btrim(display_name) <> ''"),
        ))
        .check((
            "chk_git_provider_connections_base_url",
            Expr::cust("btrim(base_url) <> ''"),
        ))
        .to_owned()
}

pub fn group_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_git_provider_connections_group")
        .table((Alias::new("context69"), Entity))
        .col(Column::GroupId)
        .col(Column::ConnectionKey)
        .to_owned()
}
