use anyhow::{Context, Result};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ActiveValue::Set, EntityTrait};

use super::entity::docling_settings;
use super::{Database, StoredDoclingSettings};

impl Database {
    pub async fn get_docling_max_inflight(&self) -> Result<usize> {
        Ok(self
            .get_docling_settings()
            .await?
            .map(|settings| settings.max_inflight)
            .unwrap_or(context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT)
            .clamp(
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX,
            ))
    }

    pub async fn get_docling_settings(&self) -> Result<Option<StoredDoclingSettings>> {
        let row = docling_settings::Entity::find_by_id(true)
            .one(&self.orm)
            .await?;
        row.map(stored_docling_settings).transpose()
    }

    pub async fn save_docling_settings(
        &self,
        settings: &StoredDoclingSettings,
    ) -> Result<StoredDoclingSettings> {
        let timeout_secs =
            i64::try_from(settings.timeout_secs).context("docling timeout is too large")?;
        let poll_interval_secs = i64::try_from(settings.poll_interval_secs)
            .context("docling poll interval is too large")?;
        let task_timeout_secs = i64::try_from(settings.task_timeout_secs)
            .context("docling task timeout is too large")?;
        let max_inflight =
            i64::try_from(settings.max_inflight).context("docling max_inflight is too large")?;

        let row = docling_settings::Entity::insert(docling_settings::ActiveModel {
            singleton: Set(true),
            base_url: Set(settings.base_url.clone()),
            timeout_secs: Set(timeout_secs),
            poll_interval_secs: Set(poll_interval_secs),
            task_timeout_secs: Set(task_timeout_secs),
            max_inflight: Set(max_inflight),
            pdf_backend: Set(settings.pdf_backend.clone()),
            images_scale: Set(settings.images_scale),
            image_export_mode: Set(settings.image_export_mode.clone()),
            do_ocr: Set(settings.do_ocr),
            force_ocr: Set(settings.force_ocr),
            ocr_engine: Set(settings.ocr_engine.clone()),
            ocr_lang: Set(settings.ocr_lang.clone()),
            do_code_enrichment: Set(settings.do_code_enrichment),
            do_formula_enrichment: Set(settings.do_formula_enrichment),
            do_picture_description: Set(settings.do_picture_description),
            openai_base_url: Set(settings.openai_base_url.clone()),
            vlm_pipeline_model: Set(settings.vlm_pipeline_model.clone()),
            picture_description_model: Set(settings.picture_description_model.clone()),
            code_formula_model: Set(settings.code_formula_model.clone()),
            picture_description_preset: Set(settings.picture_description_preset.clone()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(docling_settings::Column::Singleton)
                .update_columns([
                    docling_settings::Column::BaseUrl,
                    docling_settings::Column::TimeoutSecs,
                    docling_settings::Column::PollIntervalSecs,
                    docling_settings::Column::TaskTimeoutSecs,
                    docling_settings::Column::MaxInflight,
                    docling_settings::Column::PdfBackend,
                    docling_settings::Column::ImagesScale,
                    docling_settings::Column::ImageExportMode,
                    docling_settings::Column::DoOcr,
                    docling_settings::Column::ForceOcr,
                    docling_settings::Column::OcrEngine,
                    docling_settings::Column::OcrLang,
                    docling_settings::Column::DoCodeEnrichment,
                    docling_settings::Column::DoFormulaEnrichment,
                    docling_settings::Column::DoPictureDescription,
                    docling_settings::Column::OpenaiBaseUrl,
                    docling_settings::Column::VlmPipelineModel,
                    docling_settings::Column::PictureDescriptionModel,
                    docling_settings::Column::CodeFormulaModel,
                    docling_settings::Column::PictureDescriptionPreset,
                ])
                .value(
                    docling_settings::Column::UpdatedAt,
                    Expr::current_timestamp(),
                )
                .to_owned(),
        )
        .exec_with_returning(&self.orm)
        .await?;

        stored_docling_settings(row)
    }
}

fn stored_docling_settings(row: docling_settings::Model) -> Result<StoredDoclingSettings> {
    Ok(StoredDoclingSettings {
        base_url: row.base_url,
        timeout_secs: u64::try_from(row.timeout_secs)
            .context("docling timeout_secs must be non-negative")?,
        poll_interval_secs: u64::try_from(row.poll_interval_secs)
            .context("docling poll_interval_secs must be non-negative")?,
        task_timeout_secs: u64::try_from(row.task_timeout_secs)
            .context("docling task_timeout_secs must be non-negative")?,
        max_inflight: usize::try_from(row.max_inflight)
            .context("docling max_inflight must be non-negative")?,
        pdf_backend: row.pdf_backend,
        images_scale: row.images_scale,
        image_export_mode: row.image_export_mode,
        do_ocr: row.do_ocr,
        force_ocr: row.force_ocr,
        ocr_engine: row.ocr_engine,
        ocr_lang: row.ocr_lang,
        do_code_enrichment: row.do_code_enrichment,
        do_formula_enrichment: row.do_formula_enrichment,
        do_picture_description: row.do_picture_description,
        openai_base_url: row.openai_base_url,
        api_key: None,
        vlm_pipeline_model: row.vlm_pipeline_model,
        picture_description_model: row.picture_description_model,
        code_formula_model: row.code_formula_model,
        picture_description_preset: row.picture_description_preset,
    })
}
