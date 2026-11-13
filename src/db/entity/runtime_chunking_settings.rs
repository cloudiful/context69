use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "runtime_chunking_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub singleton: bool,
    pub max_chars: i64,
    pub overlap_chars: i64,
    #[sea_orm(column_type = "Text")]
    pub unit: String,
    #[sea_orm(column_type = "Text")]
    pub tokenizer: String,
    #[sea_orm(column_type = "Text")]
    pub tokenizer_sha256: Option<String>,
    pub max_tokens: i64,
    pub overlap_tokens: i64,
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
            ColumnDef::new(Column::Singleton)
                .boolean()
                .not_null()
                .default(true)
                .primary_key(),
        )
        .col(ColumnDef::new(Column::MaxChars).big_integer().not_null())
        .col(
            ColumnDef::new(Column::OverlapChars)
                .big_integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(Column::Unit)
                .text()
                .not_null()
                .default("chars"),
        )
        .col(
            ColumnDef::new(Column::Tokenizer)
                .text()
                .not_null()
                .default("cl100k_base"),
        )
        .col(ColumnDef::new(Column::TokenizerSha256).text())
        .col(
            ColumnDef::new(Column::MaxTokens)
                .big_integer()
                .not_null()
                .default(1024i64),
        )
        .col(
            ColumnDef::new(Column::OverlapTokens)
                .big_integer()
                .not_null()
                .default(128i64),
        )
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check((
            "runtime_chunking_settings_singleton_check",
            Expr::cust("singleton"),
        ))
        .check((
            "runtime_chunking_settings_max_chars_check",
            Expr::cust("max_chars > 0"),
        ))
        .check((
            "runtime_chunking_settings_overlap_chars_check",
            Expr::cust("overlap_chars >= 0 AND overlap_chars < max_chars"),
        ))
        .check((
            "runtime_chunking_settings_unit_check",
            Expr::cust("unit IN ('chars', 'tokens')"),
        ))
        .check((
            "runtime_chunking_settings_tokenizer_check",
            Expr::cust("tokenizer IN ('cl100k_base', 'o200k_base', 'hf_local')"),
        ))
        .check((
            "runtime_chunking_settings_max_tokens_check",
            Expr::cust("unit <> 'tokens' OR max_tokens > 0"),
        ))
        .check((
            "runtime_chunking_settings_overlap_tokens_check",
            Expr::cust("overlap_tokens >= 0 AND (unit <> 'tokens' OR overlap_tokens < max_tokens)"),
        ))
        .check((
            "runtime_chunking_settings_tokenizer_sha256_check",
            Expr::cust(
                "(tokenizer = 'hf_local' AND tokenizer_sha256 ~ '^[0-9a-f]{64}$') \
                 OR (tokenizer <> 'hf_local' AND tokenizer_sha256 IS NULL)",
            ),
        ))
        .to_owned()
}
