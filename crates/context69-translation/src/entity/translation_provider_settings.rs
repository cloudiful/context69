use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, PartialEq, DeriveEntityModel)]
#[sea_orm(
    schema_name = "context69",
    table_name = "translation_provider_settings"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub provider_key: String,
    pub enabled: bool,
    pub priority: i32,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub llm_api_kind: Option<String>,
    pub deepl_plan: Option<String>,
    pub monthly_character_limit: Option<i64>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}
impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("provider_key", &self.provider_key)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

pub fn schema() -> TableCreateStatement {
    Table::create().table((Alias::new("context69"), Entity)).if_not_exists()
        .col(ColumnDef::new(Column::ProviderKey).text().not_null().primary_key())
        .col(ColumnDef::new(Column::Enabled).boolean().not_null().default(false))
        .col(ColumnDef::new(Column::Priority).integer().not_null())
        .col(ColumnDef::new(Column::Endpoint).text())
        .col(ColumnDef::new(Column::ApiKey).text())
        .col(ColumnDef::new(Column::Model).text())
        .col(ColumnDef::new(Column::LlmApiKind).text())
        .col(ColumnDef::new(Column::DeeplPlan).text())
        .col(ColumnDef::new(Column::MonthlyCharacterLimit).big_integer())
        .col(ColumnDef::new(Column::CreatedAt).timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
        .col(ColumnDef::new(Column::UpdatedAt).timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
        .check(("chk_translation_provider_key", Expr::cust("provider_key IN ('deepl', 'llm', 'libretranslate')")))
        .check(("chk_translation_llm_api_kind", Expr::cust("llm_api_kind IS NULL OR llm_api_kind IN ('openai_responses', 'openai_chat_completions', 'anthropic_messages')")))
        .check(("chk_translation_deepl_plan", Expr::cust("deepl_plan IS NULL OR deepl_plan IN ('free', 'pro')")))
        .check(("chk_translation_monthly_limit", Expr::cust("monthly_character_limit IS NULL OR monthly_character_limit > 0")))
        .to_owned()
}
