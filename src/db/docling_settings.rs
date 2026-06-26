use anyhow::{Context, Result};

use super::{Database, DoclingSettingsRow, StoredDoclingSettings};

impl Database {
    pub async fn get_docling_settings(&self) -> Result<Option<StoredDoclingSettings>> {
        let row = sqlx::query_file_as!(
            DoclingSettingsRow,
            "src/sql/db/docling_settings/get_docling_settings.sql"
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            Ok(StoredDoclingSettings {
                base_url: row.base_url,
                timeout_secs: u64::try_from(row.timeout_secs)
                    .context("docling timeout_secs must be non-negative")?,
                poll_interval_secs: u64::try_from(row.poll_interval_secs)
                    .context("docling poll_interval_secs must be non-negative")?,
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
                provider_account_key: row.provider_account_key,
                vlm_pipeline_model: row.vlm_pipeline_model,
                picture_description_model: row.picture_description_model,
                code_formula_model: row.code_formula_model,
            })
        })
        .transpose()
    }

    pub async fn save_docling_settings(
        &self,
        settings: &StoredDoclingSettings,
    ) -> Result<StoredDoclingSettings> {
        let timeout_secs =
            i64::try_from(settings.timeout_secs).context("docling timeout is too large")?;
        let poll_interval_secs = i64::try_from(settings.poll_interval_secs)
            .context("docling poll interval is too large")?;

        let row = sqlx::query_file_as!(
            DoclingSettingsRow,
            "src/sql/db/docling_settings/save_docling_settings.sql",
            settings.base_url,
            timeout_secs,
            poll_interval_secs,
            settings.pdf_backend,
            settings.images_scale,
            settings.image_export_mode,
            settings.do_ocr,
            settings.force_ocr,
            settings.ocr_engine,
            &settings.ocr_lang,
            settings.do_code_enrichment,
            settings.do_formula_enrichment,
            settings.do_picture_description,
            settings.provider_account_key,
            settings.vlm_pipeline_model,
            settings.picture_description_model,
            settings.code_formula_model
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(StoredDoclingSettings {
            base_url: row.base_url,
            timeout_secs: u64::try_from(row.timeout_secs)
                .context("docling timeout_secs must be non-negative")?,
            poll_interval_secs: u64::try_from(row.poll_interval_secs)
                .context("docling poll_interval_secs must be non-negative")?,
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
            provider_account_key: row.provider_account_key,
            vlm_pipeline_model: row.vlm_pipeline_model,
            picture_description_model: row.picture_description_model,
            code_formula_model: row.code_formula_model,
        })
    }
}
