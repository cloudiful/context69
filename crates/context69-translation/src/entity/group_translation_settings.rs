use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "group_translation_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub group_id: i64,
    pub enabled: bool,
    pub default_target_locales: Vec<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub source_locale: Option<String>,
    pub glossary: Json,
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
            ColumnDef::new(Column::GroupId)
                .big_integer()
                .not_null()
                .primary_key(),
        )
        .col(
            ColumnDef::new(Column::Enabled)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(
            ColumnDef::new(Column::DefaultTargetLocales)
                .array(sea_orm::sea_query::ColumnType::Text)
                .not_null()
                .default(Expr::cust("ARRAY[]::TEXT[]")),
        )
        .col(ColumnDef::new(Column::SourceLocale).text())
        .col(
            ColumnDef::new(Column::Glossary)
                .json_binary()
                .not_null()
                .default(Expr::cust("'[]'::jsonb")),
        )
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
            "chk_group_translation_glossary_array",
            Expr::cust("jsonb_typeof(glossary) = 'array'"),
        ))
        .to_owned()
}
