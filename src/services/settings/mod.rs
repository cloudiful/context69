use anyhow::{Context, Result, anyhow};
use chrono::Utc;

mod mappers;
mod validate;

use self::{
    mappers::{
        config_from_stored, default_runtime_settings_response, docling_settings_from_request,
        provider_account_from_parts, provider_account_response, response_from_stored,
        runtime_settings_from_request, runtime_settings_response, search_response_from_stored,
        search_settings_from_request, unconfigured_docling_response,
    },
    validate as settings_validate,
};

use crate::{
    contracts::{
        DoclingSettingsResponse, DoclingSettingsSource, ProviderAccountResponse,
        RuntimeSettingsResponse, SearchSettingsResponse, UpdateDoclingSettingsRequest,
        UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest, UpsertProviderAccountRequest,
    },
    db::{
        Database, StoredDoclingSettings, StoredProviderAccount, StoredSearchSettings,
        default_search_settings,
    },
    docling::DoclingConfig,
    support::normalize::normalize_optional_string,
};

#[derive(Clone)]
pub struct SettingsService {
    db: Database,
}

impl SettingsService {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get_runtime_settings(&self) -> Result<RuntimeSettingsResponse> {
        Ok(self
            .db
            .get_runtime_settings()
            .await?
            .map(runtime_settings_response)
            .unwrap_or_else(default_runtime_settings_response))
    }

    pub async fn update_runtime_settings(
        &self,
        request: &UpdateRuntimeSettingsRequest,
    ) -> Result<RuntimeSettingsResponse> {
        settings_validate::runtime_settings_request(request)?;

        let stored = runtime_settings_from_request(request);
        self.ensure_provider_account_active(&stored.embedding.provider_account_key, false)
            .await?;

        let saved = self.db.save_runtime_settings(&stored).await?;
        Ok(runtime_settings_response(saved))
    }

    pub async fn list_provider_accounts(&self) -> Result<Vec<ProviderAccountResponse>> {
        Ok(self
            .db
            .list_provider_accounts()
            .await?
            .into_iter()
            .map(provider_account_response)
            .collect())
    }

    pub async fn upsert_provider_account(
        &self,
        request: &UpsertProviderAccountRequest,
    ) -> Result<ProviderAccountResponse> {
        let normalized = self.normalize_provider_account_request(request).await?;

        let saved = self
            .db
            .save_provider_account(&provider_account_from_parts(
                &normalized.account_key,
                &normalized.provider_kind,
                &normalized.display_name,
                &normalized.base_url,
                normalized.api_key,
                normalized.disabled_at,
            ))
            .await?;
        Ok(provider_account_response(saved))
    }

    pub async fn delete_provider_account(&self, account_key: &str) -> Result<()> {
        let account_key = account_key.trim();
        if account_key.is_empty() {
            return Err(anyhow!("provider account_key must not be empty"));
        }

        if let Some(runtime) = self.db.get_runtime_settings().await?
            && runtime.embedding.provider_account_key == account_key
        {
            return Err(anyhow!(
                "provider account {account_key} is referenced by embedding settings"
            ));
        }

        if let Some(docling) = self.db.get_docling_settings().await?
            && docling.provider_account_key.as_deref() == Some(account_key)
        {
            return Err(anyhow!(
                "provider account {account_key} is referenced by docling settings"
            ));
        }

        let deleted = self.db.delete_provider_account(account_key).await?;
        if !deleted {
            return Err(anyhow!("unknown provider account {account_key}"));
        }
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
        let merged_api_key = if request.vlm.clear_api_key {
            None
        } else if let Some(api_key) = normalize_optional_string(request.vlm.api_key.clone()) {
            Some(api_key)
        } else if openai_base_url.is_some() {
            existing
                .as_ref()
                .and_then(|settings| settings.api_key.clone())
        } else {
            None
        };

        let candidate = docling_settings_from_request(request, merged_api_key);
        self.validate_stored_docling_settings(&candidate).await?;

        let settings = self.db.save_docling_settings(&candidate).await?;
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
        let provider = if let Some(account_key) = settings.provider_account_key.as_deref() {
            Some(
                self.ensure_provider_account_active(account_key, true)
                    .await?,
            )
        } else {
            None
        };
        Ok(Some(config_from_stored(settings, provider)))
    }

    pub async fn ensure_provider_account_active(
        &self,
        account_key: &str,
        require_api_key: bool,
    ) -> Result<StoredProviderAccount> {
        let account = self
            .db
            .get_provider_account(account_key)
            .await?
            .with_context(|| format!("provider account {account_key} does not exist"))?;
        if account.disabled_at.is_some() {
            return Err(anyhow!("provider account {account_key} is disabled"));
        }
        if account.base_url.trim().is_empty() {
            return Err(anyhow!(
                "provider account {account_key} base_url must not be empty"
            ));
        }
        if require_api_key
            && account
                .api_key
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(anyhow!(
                "provider account {account_key} api_key is required"
            ));
        }
        Ok(account)
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

    async fn validate_stored_docling_settings(
        &self,
        settings: &StoredDoclingSettings,
    ) -> Result<()> {
        if let Some(account_key) = validate_docling_vlm_shape(settings)? {
            self.ensure_provider_account_active(account_key, true)
                .await?;
        }

        Ok(())
    }

    async fn normalize_provider_account_request(
        &self,
        request: &UpsertProviderAccountRequest,
    ) -> Result<NormalizedProviderAccountRequest> {
        let account_key = non_empty_trimmed("provider account_key", &request.account_key)?;
        let provider_kind = non_empty_trimmed("provider provider_kind", &request.provider_kind)?;
        let display_name = non_empty_trimmed("provider display_name", &request.display_name)?;
        let base_url = non_empty_trimmed("provider base_url", &request.base_url)?;

        let existing = self.db.get_provider_account(&account_key).await?;
        let api_key = if request.clear_api_key {
            None
        } else if let Some(api_key) = normalize_optional_string(request.api_key.clone()) {
            Some(api_key)
        } else {
            existing.and_then(|account| account.api_key)
        };

        Ok(NormalizedProviderAccountRequest {
            account_key,
            provider_kind,
            display_name,
            base_url,
            api_key,
            disabled_at: request.disabled.then(Utc::now),
        })
    }
}

struct NormalizedProviderAccountRequest {
    account_key: String,
    provider_kind: String,
    display_name: String,
    base_url: String,
    api_key: Option<String>,
    disabled_at: Option<chrono::DateTime<Utc>>,
}

fn non_empty_trimmed(field: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("{field} must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn validate_docling_vlm_shape(settings: &StoredDoclingSettings) -> Result<Option<&str>> {
    let provider_account_key = settings
        .provider_account_key
        .as_deref()
        .filter(|value| !value.trim().is_empty());
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

    let auth_configured = provider_account_key.is_some() || raw_auth_count == 2;
    if !auth_configured && model_count == 0 {
        return Ok(None);
    }
    if !auth_configured {
        return Err(anyhow!(
            "docling.vlm.provider_account_key or docling.vlm.openai_base_url/api_key is required when Docling VLM models are configured"
        ));
    }
    if model_count == 0 {
        return Err(anyhow!(
            "docling.vlm.vlm_pipeline_model, docling.vlm.picture_description_model, and docling.vlm.code_formula_model are required when Docling VLM is configured"
        ));
    }

    Ok(provider_account_key)
}

#[cfg(test)]
mod tests {
    use super::{
        mappers::{response_from_stored, search_response_from_stored},
        validate::{
            docling_request as validate_docling_request,
            runtime_settings_request as validate_runtime_settings_request,
            search_request as validate_search_request,
        },
        validate_docling_vlm_shape,
    };
    use crate::{
        contracts::{
            DoclingSettingsSource, RuntimeChunkingSettings, RuntimeEmbeddingSettings,
            RuntimeFileLibrarySettings, RuntimeQdrantSettings, RuntimeSchedulerSettings,
            UpdateDoclingConnectionSettings, UpdateDoclingSettingsRequest,
            UpdateDoclingVlmSettings, UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
        },
        db::{StoredDoclingSettings, default_search_settings},
    };

    fn sample_request() -> UpdateDoclingSettingsRequest {
        UpdateDoclingSettingsRequest {
            connection: UpdateDoclingConnectionSettings {
                base_url: "http://docling:5001".to_string(),
                timeout_secs: 120,
                poll_interval_secs: 2,
            },
            vlm: UpdateDoclingVlmSettings::default(),
        }
    }

    fn sample_stored() -> StoredDoclingSettings {
        StoredDoclingSettings {
            base_url: "http://docling:5001".to_string(),
            timeout_secs: 120,
            poll_interval_secs: 2,
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
            provider_account_key: None,
            openai_base_url: None,
            api_key: None,
            vlm_pipeline_model: None,
            picture_description_model: None,
            code_formula_model: None,
        }
    }

    fn sample_runtime_request() -> UpdateRuntimeSettingsRequest {
        UpdateRuntimeSettingsRequest {
            qdrant: RuntimeQdrantSettings {
                url: "http://qdrant:6334".to_string(),
                collection_name: "context69".to_string(),
                recreate_on_dimension_mismatch: false,
            },
            embedding: RuntimeEmbeddingSettings {
                provider_account_key: "embedding-default".to_string(),
                model: "text-embedding-3-large".to_string(),
                dimensions: 3072,
                timeout_secs: 30,
            },
            scheduler: RuntimeSchedulerSettings {
                interval_secs: 300,
                run_on_start: true,
                max_concurrency: 4,
                job_id: "context69-sync".to_string(),
                valkey_url: Some("redis://valkey:6379/0".to_string()),
            },
            chunking: RuntimeChunkingSettings {
                max_chars: 1200,
                overlap_chars: 200,
            },
            file_library: RuntimeFileLibrarySettings {
                storage_root: "/tmp/library".to_string(),
                max_upload_size_mb: 64,
                max_upload_request_size_mb: 128,
                ingest_concurrency: 2,
                pdf_pages_per_task: 5,
            },
        }
    }

    #[test]
    fn response_exposes_provider_reference_not_secret() {
        let mut settings = sample_stored();
        settings.provider_account_key = Some("openrouter-default".to_string());
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());

        let response = response_from_stored(DoclingSettingsSource::Database, true, settings);
        assert_eq!(
            response.vlm.provider_account_key.as_deref(),
            Some("openrouter-default")
        );
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
    fn stored_docling_settings_allow_vlm_to_be_disabled() {
        let settings = sample_stored();
        let account_key =
            validate_docling_vlm_shape(&settings).expect("disabled vlm should be valid");
        assert!(account_key.is_none());
    }

    #[test]
    fn stored_docling_settings_accept_complete_raw_vlm() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let account_key =
            validate_docling_vlm_shape(&settings).expect("raw vlm settings should be valid");
        assert!(account_key.is_none());
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
        assert!(error.to_string().contains("provider_account_key"));
    }

    #[test]
    fn stored_docling_settings_require_models_when_auth_is_present() {
        let mut settings = sample_stored();
        settings.provider_account_key = Some("openrouter-default".to_string());

        let error =
            validate_docling_vlm_shape(&settings).expect_err("auth without models should fail");
        assert!(
            error
                .to_string()
                .contains("required when Docling VLM is configured")
        );
    }

    #[test]
    fn stored_docling_settings_accept_provider_account_vlm() {
        let mut settings = sample_stored();
        settings.provider_account_key = Some("openrouter-default".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let account_key =
            validate_docling_vlm_shape(&settings).expect("provider account vlm should be valid");
        assert_eq!(account_key, Some("openrouter-default"));
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
}
