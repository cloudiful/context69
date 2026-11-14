use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "search_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub singleton: bool,
    #[sea_orm(column_type = "Text")]
    pub mode: String,
    pub rerank_enabled: bool,
    #[sea_orm(column_type = "Text")]
    pub rerank_base_url: String,
    #[sea_orm(column_type = "Text")]
    pub rerank_model: String,
    pub candidate_limit: i64,
    pub timeout_secs: i64,
    pub vector_weight: f32,
    pub keyword_weight: f32,
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
        .col(
            ColumnDef::new(Column::Mode)
                .text()
                .not_null()
                .default("hybrid"),
        )
        .col(
            ColumnDef::new(Column::RerankEnabled)
                .boolean()
                .not_null()
                .default(true),
        )
        .col(
            ColumnDef::new(Column::RerankBaseUrl)
                .text()
                .not_null()
                .default("https://openrouter.ai/api/v1"),
        )
        .col(
            ColumnDef::new(Column::RerankModel)
                .text()
                .not_null()
                .default("cohere/rerank-4-fast"),
        )
        .col(
            ColumnDef::new(Column::CandidateLimit)
                .big_integer()
                .not_null()
                .default(40i64),
        )
        .col(
            ColumnDef::new(Column::TimeoutSecs)
                .big_integer()
                .not_null()
                .default(10i64),
        )
        .col(
            ColumnDef::new(Column::VectorWeight)
                .float()
                .not_null()
                .default(0.55f32),
        )
        .col(
            ColumnDef::new(Column::KeywordWeight)
                .float()
                .not_null()
                .default(0.35f32),
        )
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check(("search_settings_singleton_check", Expr::cust("singleton")))
        .check((
            "search_settings_mode_check",
            Expr::cust("mode IN ('vector', 'hybrid')"),
        ))
        .check((
            "search_settings_rerank_base_url_check",
            Expr::cust("btrim(rerank_base_url) <> ''"),
        ))
        .check((
            "search_settings_rerank_model_check",
            Expr::cust("btrim(rerank_model) <> ''"),
        ))
        .check((
            "search_settings_candidate_limit_check",
            Expr::cust("candidate_limit > 0"),
        ))
        .check((
            "search_settings_timeout_secs_check",
            Expr::cust("timeout_secs > 0"),
        ))
        .to_owned()
}
