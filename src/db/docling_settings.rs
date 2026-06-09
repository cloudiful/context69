use anyhow::{Context, Result};

use super::{Database, DoclingSettingsRow, StoredDoclingSettings};

impl Database {
    pub async fn get_docling_settings(&self) -> Result<Option<StoredDoclingSettings>> {
        let row = sqlx::query_as::<_, DoclingSettingsRow>(
            r#"
            SELECT
                base_url,
                timeout_secs,
                poll_interval_secs,
                pdf_backend,
                images_scale,
                image_export_mode,
                do_ocr,
                force_ocr,
                ocr_engine,
                ocr_lang,
                do_code_enrichment,
                do_formula_enrichment,
                do_picture_description,
                provider_account_key,
                vlm_pipeline_model,
                picture_description_model,
                code_formula_model
            FROM context69.docling_settings
            WHERE singleton = TRUE
            "#,
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
        let row = sqlx::query_as::<_, DoclingSettingsRow>(
            r#"
            INSERT INTO context69.docling_settings (
                singleton,
                base_url,
                timeout_secs,
                poll_interval_secs,
                pdf_backend,
                images_scale,
                image_export_mode,
                do_ocr,
                force_ocr,
                ocr_engine,
                ocr_lang,
                do_code_enrichment,
                do_formula_enrichment,
                do_picture_description,
                provider_account_key,
                vlm_pipeline_model,
                picture_description_model,
                code_formula_model,
                updated_at
            )
            VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, now())
            ON CONFLICT (singleton) DO UPDATE
            SET base_url = EXCLUDED.base_url,
                timeout_secs = EXCLUDED.timeout_secs,
                poll_interval_secs = EXCLUDED.poll_interval_secs,
                pdf_backend = EXCLUDED.pdf_backend,
                images_scale = EXCLUDED.images_scale,
                image_export_mode = EXCLUDED.image_export_mode,
                do_ocr = EXCLUDED.do_ocr,
                force_ocr = EXCLUDED.force_ocr,
                ocr_engine = EXCLUDED.ocr_engine,
                ocr_lang = EXCLUDED.ocr_lang,
                do_code_enrichment = EXCLUDED.do_code_enrichment,
                do_formula_enrichment = EXCLUDED.do_formula_enrichment,
                do_picture_description = EXCLUDED.do_picture_description,
                provider_account_key = EXCLUDED.provider_account_key,
                vlm_pipeline_model = EXCLUDED.vlm_pipeline_model,
                picture_description_model = EXCLUDED.picture_description_model,
                code_formula_model = EXCLUDED.code_formula_model,
                updated_at = now()
            RETURNING
                base_url,
                timeout_secs,
                poll_interval_secs,
                pdf_backend,
                images_scale,
                image_export_mode,
                do_ocr,
                force_ocr,
                ocr_engine,
                ocr_lang,
                do_code_enrichment,
                do_formula_enrichment,
                do_picture_description,
                provider_account_key,
                vlm_pipeline_model,
                picture_description_model,
                code_formula_model
            "#,
        )
        .bind(&settings.base_url)
        .bind(i64::try_from(settings.timeout_secs).context("docling timeout is too large")?)
        .bind(
            i64::try_from(settings.poll_interval_secs)
                .context("docling poll interval is too large")?,
        )
        .bind(&settings.pdf_backend)
        .bind(settings.images_scale)
        .bind(&settings.image_export_mode)
        .bind(settings.do_ocr)
        .bind(settings.force_ocr)
        .bind(&settings.ocr_engine)
        .bind(&settings.ocr_lang)
        .bind(settings.do_code_enrichment)
        .bind(settings.do_formula_enrichment)
        .bind(settings.do_picture_description)
        .bind(&settings.provider_account_key)
        .bind(&settings.vlm_pipeline_model)
        .bind(&settings.picture_description_model)
        .bind(&settings.code_formula_model)
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
