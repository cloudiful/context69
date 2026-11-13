use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Alias, ColumnDef, Expr, Table, TableCreateStatement};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(schema_name = "context69", table_name = "docling_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub singleton: bool,
    #[sea_orm(column_type = "Text")]
    pub base_url: String,
    pub timeout_secs: i64,
    pub poll_interval_secs: i64,
    pub task_timeout_secs: i64,
    pub max_inflight: i64,
    #[sea_orm(column_type = "Text")]
    pub pdf_backend: Option<String>,
    pub images_scale: Option<f64>,
    #[sea_orm(column_type = "Text")]
    pub image_export_mode: Option<String>,
    pub do_ocr: bool,
    pub force_ocr: bool,
    #[sea_orm(column_type = "Text")]
    pub ocr_engine: Option<String>,
    pub ocr_lang: Vec<String>,
    pub do_code_enrichment: bool,
    pub do_formula_enrichment: bool,
    pub do_picture_description: bool,
    #[sea_orm(column_type = "Text")]
    pub openai_base_url: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub vlm_pipeline_model: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub picture_description_model: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub code_formula_model: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub picture_description_preset: Option<String>,
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
        .col(ColumnDef::new(Column::BaseUrl).text().not_null())
        .col(ColumnDef::new(Column::TimeoutSecs).big_integer().not_null())
        .col(
            ColumnDef::new(Column::PollIntervalSecs)
                .big_integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(Column::TaskTimeoutSecs)
                .big_integer()
                .not_null()
                .default(600i64),
        )
        .col(
            ColumnDef::new(Column::MaxInflight)
                .big_integer()
                .not_null()
                .default(2i64),
        )
        .col(ColumnDef::new(Column::PdfBackend).text())
        .col(ColumnDef::new(Column::ImagesScale).double())
        .col(ColumnDef::new(Column::ImageExportMode).text())
        .col(
            ColumnDef::new(Column::DoOcr)
                .boolean()
                .not_null()
                .default(true),
        )
        .col(
            ColumnDef::new(Column::ForceOcr)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(ColumnDef::new(Column::OcrEngine).text())
        .col(
            ColumnDef::new(Column::OcrLang)
                .array(ColumnType::Text)
                .not_null()
                .default(Expr::cust("'{}'")),
        )
        .col(
            ColumnDef::new(Column::DoCodeEnrichment)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(
            ColumnDef::new(Column::DoFormulaEnrichment)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(
            ColumnDef::new(Column::DoPictureDescription)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(ColumnDef::new(Column::OpenaiBaseUrl).text())
        .col(ColumnDef::new(Column::VlmPipelineModel).text())
        .col(ColumnDef::new(Column::PictureDescriptionModel).text())
        .col(ColumnDef::new(Column::CodeFormulaModel).text())
        .col(ColumnDef::new(Column::PictureDescriptionPreset).text())
        .col(
            ColumnDef::new(Column::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .check(("docling_settings_singleton_check", Expr::cust("singleton")))
        .check((
            "docling_settings_base_url_check",
            Expr::cust("btrim(base_url) <> ''"),
        ))
        .check((
            "docling_settings_timeout_secs_check",
            Expr::cust("timeout_secs > 0"),
        ))
        .check((
            "docling_settings_poll_interval_secs_check",
            Expr::cust("poll_interval_secs > 0"),
        ))
        .check((
            "docling_settings_task_timeout_secs_check",
            Expr::cust("task_timeout_secs > 0"),
        ))
        .check((
            "docling_settings_max_inflight_check",
            Expr::cust("max_inflight >= 1 AND max_inflight <= 32"),
        ))
        .to_owned()
}
