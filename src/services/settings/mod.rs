use std::sync::Arc;

use anyhow::{Context, Result, anyhow};

mod mappers;
mod runtime_mappers;
mod validate;

use self::{
    mappers::{
        config_from_stored, docling_settings_from_request, response_from_stored,
        search_response_from_stored, search_settings_from_request, unconfigured_docling_response,
    },
    runtime_mappers::{
        default_runtime_settings_response, runtime_settings_from_request, runtime_settings_response,
    },
    validate as settings_validate,
};

use crate::{
    contracts::{
        DoclingSettingsResponse, DoclingSettingsSource, RuntimeSettingsResponse,
        SearchSettingsResponse, UpdateDoclingSettingsRequest, UpdateRuntimeSettingsRequest,
        UpdateSearchSettingsRequest,
    },
    db::{Database, StoredDoclingSettings, StoredSearchSettings, default_search_settings},
    docling::DoclingConfig,
    support::normalize::normalize_optional_string,
};

#[derive(Clone)]
pub struct SettingsService {
    db: Database,
    docling_settings_observer: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl SettingsService {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            docling_settings_observer: None,
        }
    }

    /// Register a hook invoked after Docling settings are saved, so dependency
    /// gates (e.g. docling readiness) refresh without a process restart.
    pub fn set_docling_settings_observer(&mut self, observer: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.docling_settings_observer = observer;
    }

    pub async fn get_runtime_settings(&self) -> Result<RuntimeSettingsResponse> {
        Ok(self
            .db
            .get_runtime_settings()
            .await?
            .map(runtime_settings_response)
            .unwrap_or_else(default_runtime_settings_response))
    }

    pub async fn trusted_proxy_enabled(&self) -> Result<bool> {
        Ok(self
            .db
            .get_runtime_settings()
            .await?
            .is_some_and(|settings| settings.file_library.trusted_proxy_enabled))
    }

    pub async fn update_runtime_settings(
        &self,
        request: &UpdateRuntimeSettingsRequest,
    ) -> Result<RuntimeSettingsResponse> {
        settings_validate::runtime_settings_request(request)?;

        let existing = self.db.get_runtime_settings().await?;
        let api_key =
            if let Some(api_key) = normalize_optional_string(request.embedding.api_key.clone()) {
                Some(api_key)
            } else {
                existing
                    .as_ref()
                    .and_then(|settings| settings.embedding.api_key.clone())
            };
        let mut stored = runtime_settings_from_request(request, api_key);
        if let Some(s3) = stored.file_library.s3.as_mut()
            && s3.secret_key.is_empty()
        {
            s3.secret_key = existing
                .and_then(|settings| settings.file_library.s3)
                .map(|s3| s3.secret_key)
                .unwrap_or_default();
        }
        if stored
            .file_library
            .s3
            .as_ref()
            .is_some_and(|s3| s3.secret_key.is_empty())
        {
            return Err(anyhow!(
                "runtime.file_library.s3.secret_key must not be empty"
            ));
        }

        let saved = self.db.save_runtime_settings(&stored).await?;
        Ok(runtime_settings_response(saved))
    }

    pub async fn test_s3_connection(
        &self,
        request: &crate::contracts::UpdateRuntimeS3Settings,
    ) -> Result<()> {
        let existing = self.db.get_runtime_settings().await?;
        let secret_key = normalize_optional_string(request.secret_key.clone())
            .or_else(|| {
                existing
                    .and_then(|settings| settings.file_library.s3)
                    .map(|s3| s3.secret_key)
            })
            .context("runtime.file_library.s3.secret_key must not be empty")?;
        let config = crate::config::S3StorageConfig {
            endpoint: request.endpoint.trim().to_string(),
            region: request.region.trim().to_string(),
            bucket: request.bucket.trim().to_string(),
            prefix: request.prefix.trim_matches('/').to_string(),
            path_style: request.path_style,
            access_key: request.access_key.trim().to_string(),
            secret_key,
        };
        crate::services::library::object_storage::LibraryObjectStorage::from_s3(&config)?
            .check()
            .await
    }

    pub async fn test_valkey_connection(
        &self,
        request: &crate::contracts::TestRuntimeValkeyRequest,
    ) -> Result<()> {
        let valkey_url = request.valkey_url.trim();
        if valkey_url.is_empty() {
            return Err(anyhow!("runtime.scheduler.valkey_url must not be empty"));
        }

        let client =
            redis::Client::open(valkey_url).context("invalid runtime.scheduler.valkey_url")?;
        let mut connection = client
            .get_connection_manager()
            .await
            .context("failed to connect to Valkey")?;
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .context("Valkey PING failed")?;
        Ok(())
    }

    pub async fn get_docling_settings(&self) -> Result<DoclingSettingsResponse> {
        if let Some(settings) = self.db.get_docling_settings().await? {
            return Ok(response_from_stored(
                DoclingSettingsSource::Database,
                true,
                settings,
            ));
        }

        Ok(unconfigured_docling_response())
    }

    pub async fn update_docling_settings(
        &self,
        request: &UpdateDoclingSettingsRequest,
    ) -> Result<DoclingSettingsResponse> {
        settings_validate::docling_request(request)?;

        let existing = self.db.get_docling_settings().await?;
        let openai_base_url = normalize_optional_string(request.vlm.openai_base_url.clone());
        let merged_api_key =
            if let Some(api_key) = normalize_optional_string(request.vlm.api_key.clone()) {
                Some(api_key)
            } else if openai_base_url.is_some() {
                existing
                    .as_ref()
                    .and_then(|settings| settings.api_key.clone())
            } else {
                None
            };

        let candidate = docling_settings_from_request(request, merged_api_key);
        validate_docling_vlm_shape(&candidate)?;

        let settings = self.db.save_docling_settings(&candidate).await?;
        if let Some(observer) = &self.docling_settings_observer {
            observer();
        }
        Ok(response_from_stored(
            DoclingSettingsSource::Database,
            true,
            settings,
        ))
    }

    pub async fn resolve_docling_config(&self) -> Result<Option<DoclingConfig>> {
        let Some(settings) = self.db.get_docling_settings().await? else {
            return Ok(None);
        };
        Ok(Some(config_from_stored(settings)))
    }

    pub async fn get_search_settings(&self) -> Result<SearchSettingsResponse> {
        let settings = self
            .db
            .get_search_settings()
            .await?
            .unwrap_or_else(default_search_settings);
        Ok(search_response_from_stored(settings))
    }

    pub async fn update_search_settings(
        &self,
        request: &UpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse> {
        settings_validate::search_request(request)?;

        let existing = self.db.get_search_settings().await?;
        let merged_api_key = if request.clear_api_key {
            None
        } else if let Some(api_key) = normalize_optional_string(request.api_key.clone()) {
            Some(api_key)
        } else {
            existing
                .as_ref()
                .and_then(|settings| settings.api_key.clone())
        };

        let candidate = search_settings_from_request(request, merged_api_key);
        settings_validate::stored_search_settings(&candidate)?;

        let settings = self.db.save_search_settings(&candidate).await?;
        Ok(search_response_from_stored(settings))
    }

    pub async fn resolve_search_settings(&self) -> Result<StoredSearchSettings> {
        Ok(self
            .db
            .get_search_settings()
            .await?
            .unwrap_or_else(default_search_settings))
    }
}

fn validate_docling_vlm_shape(settings: &StoredDoclingSettings) -> Result<()> {
    let openai_base_url = settings
        .openai_base_url
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let api_key = settings
        .api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let vlm_pipeline_model = settings
        .vlm_pipeline_model
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let picture_description_model = settings
        .picture_description_model
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let code_formula_model = settings
        .code_formula_model
        .as_deref()
        .filter(|value| !value.trim().is_empty());

    let raw_auth_count = [openai_base_url, api_key]
        .into_iter()
        .filter(Option::is_some)
        .count();
    if raw_auth_count == 1 {
        return Err(anyhow!(
            "docling.vlm.openai_base_url and docling.vlm.api_key must be configured together"
        ));
    }

    let model_count = [
        vlm_pipeline_model,
        picture_description_model,
        code_formula_model,
    ]
    .into_iter()
    .filter(Option::is_some)
    .count();
    if model_count != 0 && model_count != 3 {
        return Err(anyhow!(
            "docling.vlm model fields must be fully configured together: vlm_pipeline_model, picture_description_model, code_formula_model"
        ));
    }

    let auth_configured = raw_auth_count == 2;
    if !auth_configured && model_count == 0 {
        return Ok(());
    }
    if !auth_configured {
        return Err(anyhow!(
            "docling.vlm.openai_base_url and docling.vlm.api_key are required when Docling VLM models are configured"
        ));
    }
    if model_count == 0 {
        return Err(anyhow!(
            "docling.vlm.vlm_pipeline_model, docling.vlm.picture_description_model, and docling.vlm.code_formula_model are required when Docling VLM is configured"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        mappers::{
            config_from_stored, docling_settings_from_request, response_from_stored,
            search_response_from_stored,
        },
        runtime_mappers::{runtime_settings_from_request, runtime_settings_response},
        validate::{
            docling_request as validate_docling_request,
            runtime_settings_request as validate_runtime_settings_request,
            search_request as validate_search_request,
        },
        validate_docling_vlm_shape,
    };
    use crate::{
        contracts::{
            DoclingSettingsSource, RuntimeChunkingSettings, RuntimeQdrantSettings,
            RuntimeSchedulerSettings, UpdateDoclingConnectionSettings,
            UpdateDoclingSettingsRequest, UpdateDoclingVlmSettings, UpdateRuntimeEmbeddingSettings,
            UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
        },
        db::{StoredDoclingSettings, default_search_settings},
    };

    fn sample_request() -> UpdateDoclingSettingsRequest {
        UpdateDoclingSettingsRequest {
            connection: UpdateDoclingConnectionSettings {
                base_url: "http://docling:5001".to_string(),
                timeout_secs: 120,
                poll_interval_secs: 2,
                task_timeout_secs: 3600,
            },
            vlm: UpdateDoclingVlmSettings::default(),
        }
    }

    fn sample_stored() -> StoredDoclingSettings {
        StoredDoclingSettings {
            base_url: "http://docling:5001".to_string(),
            timeout_secs: 120,
            poll_interval_secs: 2,
            task_timeout_secs: 3600,
            pdf_backend: None,
            images_scale: None,
            image_export_mode: None,
            do_ocr: true,
            force_ocr: false,
            ocr_engine: None,
            ocr_lang: Vec::new(),
            do_code_enrichment: false,
            do_formula_enrichment: false,
            do_picture_description: false,
            openai_base_url: None,
            api_key: None,
            vlm_pipeline_model: None,
            picture_description_model: None,
            code_formula_model: None,
            picture_description_preset: None,
        }
    }

    fn sample_runtime_request() -> UpdateRuntimeSettingsRequest {
        UpdateRuntimeSettingsRequest {
            qdrant: RuntimeQdrantSettings {
                url: "http://qdrant:6334".to_string(),
                collection_name: "context69".to_string(),
                recreate_on_dimension_mismatch: false,
            },
            embedding: UpdateRuntimeEmbeddingSettings {
                base_url: "https://openrouter.ai/api/v1".to_string(),
                model: "text-embedding-3-large".to_string(),
                dimensions: 3072,
                timeout_secs: 30,
                api_key: None,
            },
            scheduler: RuntimeSchedulerSettings {
                interval_secs: 300,
                run_on_start: true,
                max_concurrency: 2,
                job_id: "context69-sync".to_string(),
                valkey_url: Some("redis://valkey:6379/0".to_string()),
            },
            chunking: RuntimeChunkingSettings {
                max_chars: 1200,
                overlap_chars: 200,
            },
            file_library: crate::contracts::UpdateRuntimeFileLibrarySettings {
                storage_root: "/tmp/library".to_string(),
                max_upload_size_mb: 128,
                max_upload_request_size_mb: 128,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
        }
    }

    #[test]
    fn docling_response_hides_api_key() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());

        let response = response_from_stored(DoclingSettingsSource::Database, true, settings);
        assert_eq!(
            response.vlm.openai_base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(response.vlm.has_api_key);
    }

    #[test]
    fn request_allows_docling_vlm_to_be_disabled() {
        validate_docling_request(&sample_request()).expect("request without vlm should be valid");
    }

    #[test]
    fn request_rejects_zero_docling_task_timeout() {
        let mut request = sample_request();
        request.connection.task_timeout_secs = 0;

        let error =
            validate_docling_request(&request).expect_err("zero task timeout should be rejected");
        assert!(error.to_string().contains("task_timeout_secs"));
    }

    #[test]
    fn stored_docling_settings_allow_vlm_to_be_disabled() {
        let settings = sample_stored();
        validate_docling_vlm_shape(&settings).expect("disabled vlm should be valid");
    }

    #[test]
    fn stored_docling_settings_accept_complete_raw_vlm() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        validate_docling_vlm_shape(&settings).expect("raw vlm settings should be valid");
    }

    #[test]
    fn stored_docling_settings_require_complete_raw_auth_fields() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let error =
            validate_docling_vlm_shape(&settings).expect_err("partial raw auth should be invalid");
        assert!(error.to_string().contains("openai_base_url"));
        assert!(error.to_string().contains("api_key"));
    }

    #[test]
    fn stored_docling_settings_require_auth_when_models_are_present() {
        let mut settings = sample_stored();
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let error =
            validate_docling_vlm_shape(&settings).expect_err("models without auth should fail");
        assert!(error.to_string().contains("openai_base_url"));
    }

    #[test]
    fn stored_docling_settings_require_models_when_auth_is_present() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());

        let error =
            validate_docling_vlm_shape(&settings).expect_err("auth without models should fail");
        assert!(
            error
                .to_string()
                .contains("required when Docling VLM is configured")
        );
    }

    #[test]
    fn runtime_request_rejects_invalid_chunking() {
        let mut request = sample_runtime_request();
        request.chunking.overlap_chars = request.chunking.max_chars;

        let error =
            validate_runtime_settings_request(&request).expect_err("runtime request should fail");
        assert!(error.to_string().contains("overlap_chars"));
    }

    #[test]
    fn runtime_request_rejects_invalid_url_import_limits() {
        let mut request = sample_runtime_request();
        request.file_library.url_import_concurrency = 0;

        let error = validate_runtime_settings_request(&request)
            .expect_err("zero URL import workers should be rejected");
        assert!(error.to_string().contains("url_import_concurrency"));

        request.file_library.url_import_concurrency = 1;
        request.file_library.url_import_min_interval_ms = 0;
        let error = validate_runtime_settings_request(&request)
            .expect_err("zero URL import interval should be rejected");
        assert!(error.to_string().contains("url_import_min_interval_ms"));
    }

    #[test]
    fn trusted_proxy_setting_round_trips_and_defaults_off() {
        let mut request = sample_runtime_request();
        request.file_library.trusted_proxy_enabled = true;

        let stored = runtime_settings_from_request(&request, None);
        assert!(stored.file_library.trusted_proxy_enabled);
        assert!(
            runtime_settings_response(stored)
                .file_library
                .trusted_proxy_enabled
        );
        assert!(
            !crate::config::Config::default()
                .file_library
                .trusted_proxy_enabled
        );
    }

    #[test]
    fn search_response_hides_api_key() {
        let mut settings = default_search_settings();
        settings.api_key = Some("secret".to_string());

        let response = search_response_from_stored(settings);

        assert!(response.has_api_key);
    }

    #[test]
    fn search_request_rejects_empty_rerank_model() {
        let request = UpdateSearchSettingsRequest {
            mode: crate::contracts::SearchMode::Hybrid,
            rerank_enabled: true,
            rerank_base_url: "https://openrouter.ai/api/v1".to_string(),
            rerank_model: " ".to_string(),
            candidate_limit: 40,
            timeout_secs: 10,
            api_key: None,
            clear_api_key: false,
        };

        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("rerank_model"));
    }

    #[test]
    fn picture_description_preset_round_trips_through_request_and_stored_config() {
        let mut request = sample_request();
        request.vlm = UpdateDoclingVlmSettings {
            picture_description_preset: Some("smolvlm".to_string()),
            ..UpdateDoclingVlmSettings::default()
        };

        let stored = docling_settings_from_request(&request, None);
        assert_eq!(
            stored.picture_description_preset.as_deref(),
            Some("smolvlm"),
            "settings mapper must persist picture_description_preset from request to stored"
        );

        let config = config_from_stored(stored.clone());
        assert_eq!(
            config.vlm.picture_description_preset.as_deref(),
            Some("smolvlm"),
            "stored -> runtime config must surface picture_description_preset"
        );

        let response = response_from_stored(DoclingSettingsSource::Database, true, stored);
        assert_eq!(
            response.vlm.picture_description_preset.as_deref(),
            Some("smolvlm"),
            "response must include picture_description_preset without affecting has_api_key"
        );
        assert!(
            !response.vlm.has_api_key,
            "preset-only settings must not pretend to have an api key"
        );
    }

    #[test]
    fn empty_picture_description_preset_is_normalized_away_in_mapper() {
        let request = sample_request();
        let stored = docling_settings_from_request(&request, None);
        assert!(
            stored.picture_description_preset.is_none(),
            "blank picture_description_preset must be normalized away by the mapper"
        );
    }
}
