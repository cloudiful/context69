use anyhow::Result;

use crate::{
    docling::DoclingConfig,
    support::normalize::{normalize_optional_string, normalize_string_list},
};

use super::types::{ConnectionConfig, SchedulerConfig, SourceConfig};

pub(super) fn normalize_scheduler_config(mut value: SchedulerConfig) -> SchedulerConfig {
    value.valkey_url = normalize_optional_string(value.valkey_url);
    value
}

pub(super) fn normalize_connection_config(mut value: ConnectionConfig) -> ConnectionConfig {
    value.name = value.name.trim().to_string();
    value.database_url = value.database_url.trim().to_string();
    value
}

pub(super) fn normalize_source_config(mut value: SourceConfig) -> Result<SourceConfig> {
    value.display_name = normalize_optional_string(value.display_name);
    value.description = normalize_optional_string(value.description);
    value.key = value.key.trim().to_string();
    value.connection = value.connection.trim().to_string();
    value.connector.base_query = value.connector.base_query.trim().to_string();
    value.example_queries = normalize_string_list(value.example_queries)
        .into_iter()
        .fold(Vec::new(), |mut acc, query| {
            if !acc.contains(&query) {
                acc.push(query);
            }
            acc
        });
    Ok(value)
}

pub(super) fn normalize_docling_config(mut value: DoclingConfig) -> DoclingConfig {
    value.connection.base_url = value.connection.base_url.trim().to_string();
    value.conversion.pdf_backend = normalize_optional_string(value.conversion.pdf_backend);
    value.conversion.image_export_mode =
        normalize_optional_string(value.conversion.image_export_mode);
    value.ocr.ocr_engine = normalize_optional_string(value.ocr.ocr_engine);
    value.ocr.ocr_lang = normalize_string_list(value.ocr.ocr_lang);
    value.vlm.openai_base_url = normalize_optional_string(value.vlm.openai_base_url);
    value.vlm.api_key = normalize_optional_string(value.vlm.api_key);
    value.vlm.vlm_pipeline_model = normalize_optional_string(value.vlm.vlm_pipeline_model);
    value.vlm.picture_description_model =
        normalize_optional_string(value.vlm.picture_description_model);
    value.vlm.code_formula_model = normalize_optional_string(value.vlm.code_formula_model);
    value
}
