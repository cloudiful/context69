use std::sync::Arc;

use anyhow::Result;

use crate::domain_errors::DomainError;

mod mappers;
mod runtime_mappers;
pub(crate) mod secrets;
mod validate;

use self::{
    mappers::{
        canonical_search_settings_from_request, config_from_stored, docling_settings_from_request,
        response_from_stored, search_response_from_stored, search_settings_from_request,
        unconfigured_docling_response, validate_docling_vlm_shape,
    },
    runtime_mappers::{
        default_runtime_settings_response, runtime_settings_from_request,
        runtime_settings_response, s3_secret_patch,
    },
    secrets::{SettingsSecrets, docling_vlm_patch, optional_key_patch},
    validate as settings_validate,
};

use crate::{
    contracts::{
        CanonicalUpdateSearchSettingsRequest, DoclingSettingsResponse, DoclingSettingsSource,
        RuntimeSettingsResponse, SearchSettingsResponse, UpdateDoclingSettingsRequest,
        UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
    },
    db::{Database, default_search_settings},
    docling::DoclingConfig,
    services::secret_store::{self, SecretStore},
    support::normalize::normalize_optional_string,
};

#[derive(Clone)]
pub struct SettingsService {
    db: Database,
    search_secrets: SettingsSecrets,
    embedding_secrets: SettingsSecrets,
    docling_secrets: SettingsSecrets,
    s3_secrets: SettingsSecrets,
    docling_settings_observer: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl SettingsService {
    /// Builds the service on a store with no master key.
    ///
    /// Every API key then round-trips through the shared store in its legacy
    /// plaintext representation and is read back from the legacy column, which is
    /// exactly the behaviour of a deployment that has not configured a master
    /// key. The application path is [`Self::with_secrets`], which wires the
    /// configured store; this constructor exists for callers that have no
    /// deployment configuration to hand, and it uses one code path rather than a
    /// bypass.
    pub fn new(db: Database) -> Self {
        Self::with_secrets(db.clone(), secret_store::build_unkeyed(&db))
    }

    /// Builds the service on the configured shared store.
    ///
    /// The API-key categories each get their own binding, so a call site never
    /// names a secret by a string literal.
    pub fn with_secrets(db: Database, store: SecretStore) -> Self {
        Self {
            db,
            search_secrets: SettingsSecrets::search(store.clone()),
            embedding_secrets: SettingsSecrets::embedding(store.clone()),
            docling_secrets: SettingsSecrets::docling(store.clone()),
            s3_secrets: SettingsSecrets::runtime_s3(store),
            docling_settings_observer: None,
        }
    }

    /// Register a hook invoked after Docling settings are saved, so dependency
    /// gates (e.g. docling readiness) refresh without a process restart.
    pub fn set_docling_settings_observer(&mut self, observer: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.docling_settings_observer = observer;
    }

    pub async fn get_runtime_settings(&self) -> Result<RuntimeSettingsResponse> {
        let Some(stored) = self.db.get_runtime_settings().await? else {
            return Ok(default_runtime_settings_response());
        };
        // Presence only: a settings projection never opens a stored key, so it
        // stays truthful on a deployment that cannot decrypt.
        let has = self.embedding_secrets.is_present().await?;
        let has_s3_secret_key = match stored.file_library.s3.as_ref() {
            Some(_) => self.s3_secrets.is_present().await?,
            None => false,
        };
        Ok(runtime_settings_response(stored, has, has_s3_secret_key))
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

        let patch = optional_key_patch(request.embedding.api_key.clone());
        // Resolved before anything is written even though the value is not part
        // of the row: a save must fail closed on a sealed key this deployment
        // cannot open, exactly as a read does.
        self.embedding_secrets.resolve_and_merge(&patch).await?;

        // The S3 secret key follows the same contract as the API keys, and it is
        // the only way this request learns whether a key is in effect: a supplied
        // one replaces it, an absent or blank one keeps the stored one.
        let s3_patch = s3_secret_patch(request.file_library.s3.as_ref());
        let s3_secret_key = self.s3_secrets.resolve_and_merge(&s3_patch).await?;

        let stored = runtime_settings_from_request(request);
        if stored.file_library.s3.is_some() && s3_secret_key.is_none() {
            return Err(DomainError::invalid_argument(
                "runtime.file_library.s3.secret_key must not be empty",
            )
            .into());
        }

        // Committed only once the candidate is known to be valid, so a rejected
        // request leaves the stored keys as they were.
        self.embedding_secrets.commit(&patch).await?;
        self.s3_secrets.commit(&s3_patch).await?;
        let saved = self.db.save_runtime_settings(&stored).await?;
        let has = self.embedding_secrets.is_present().await?;
        let has_s3_secret_key = match saved.file_library.s3.as_ref() {
            Some(_) => self.s3_secrets.is_present().await?,
            None => false,
        };
        Ok(runtime_settings_response(saved, has, has_s3_secret_key))
    }

    pub async fn test_s3_connection(
        &self,
        request: &crate::contracts::UpdateRuntimeS3Settings,
    ) -> Result<()> {
        // A probe never writes. A supplied key is used as given, and otherwise
        // the stored one is resolved through the store, which fails closed
        // rather than reporting no credential.
        let secret_key = match normalize_optional_string(request.secret_key.clone()) {
            Some(supplied) => supplied,
            None => self
                .s3_secrets
                .resolve()
                .await?
                .ok_or_else(|| {
                    DomainError::invalid_argument(
                        "runtime.file_library.s3.secret_key must not be empty",
                    )
                })
                .map_err(anyhow::Error::from)?,
        };
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
            return Err(DomainError::invalid_argument(
                "runtime.scheduler.valkey_url must not be empty",
            )
            .into());
        }

        let client = redis::Client::open(valkey_url).map_err(|error| {
            DomainError::invalid_argument(format!("invalid runtime.scheduler.valkey_url: {error}"))
        })?;
        let mut connection = client.get_connection_manager().await.map_err(|error| {
            DomainError::internal(format!("failed to connect to Valkey: {error}"))
        })?;
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .map_err(|error| DomainError::internal(format!("Valkey PING failed: {error}")))?;
        Ok(())
    }

    pub async fn get_docling_settings(&self) -> Result<DoclingSettingsResponse> {
        let Some(settings) = self.db.get_docling_settings().await? else {
            return Ok(unconfigured_docling_response());
        };
        let has = self.docling_secrets.is_present().await?;
        Ok(response_from_stored(
            DoclingSettingsSource::Database,
            true,
            settings,
            has,
        ))
    }

    pub async fn update_docling_settings(
        &self,
        request: &UpdateDoclingSettingsRequest,
    ) -> Result<DoclingSettingsResponse> {
        settings_validate::docling_request(request)?;

        let patch = docling_vlm_patch(
            request.vlm.api_key.clone(),
            normalize_optional_string(request.vlm.openai_base_url.clone()).is_some(),
        );
        let candidate = self
            .docling_secrets
            .stage(
                &patch,
                |api_key| docling_settings_from_request(request, api_key),
                validate_docling_vlm_shape,
            )
            .await?;

        let settings = self.db.save_docling_settings(&candidate).await?;
        if let Some(observer) = &self.docling_settings_observer {
            observer();
        }
        let has = self.docling_secrets.is_present().await?;
        Ok(response_from_stored(
            DoclingSettingsSource::Database,
            true,
            settings,
            has,
        ))
    }

    /// The Docling runtime config, with the API key resolved through the store.
    ///
    /// The value is handed to the provider in memory and is never logged, put in
    /// a task payload, or returned by an API.
    pub async fn resolve_docling_config(&self) -> Result<Option<DoclingConfig>> {
        let Some(settings) = self.db.get_docling_settings().await? else {
            return Ok(None);
        };
        let api_key = self.docling_secrets.resolve().await?;
        Ok(Some(config_from_stored(settings, api_key)))
    }

    pub async fn get_search_settings(&self) -> Result<SearchSettingsResponse> {
        let settings = self
            .db
            .get_search_settings()
            .await?
            .unwrap_or_else(default_search_settings);
        // Presence only: a settings projection never opens the stored key.
        let has = self.search_secrets.is_present().await?;
        Ok(search_response_from_stored(settings, has))
    }

    pub async fn update_search_settings(
        &self,
        request: &CanonicalUpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse> {
        settings_validate::canonical_search_request(request)?;

        let candidate = self
            .search_secrets
            .stage(
                &request.api_key,
                |api_key| canonical_search_settings_from_request(request, api_key),
                settings_validate::stored_search_settings,
            )
            .await?;

        let settings = self.db.save_search_settings(&candidate).await?;
        let has = self.search_secrets.is_present().await?;
        Ok(search_response_from_stored(settings, has))
    }

    /// Legacy wire-compat entry: validates and maps through the legacy
    /// wrappers (which delegate to the canonical kernel) without behavior
    /// change.
    pub async fn update_search_settings_legacy(
        &self,
        request: &UpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse> {
        settings_validate::search_request(request)?;

        let canonical = CanonicalUpdateSearchSettingsRequest::from(request.clone());
        let candidate = self
            .search_secrets
            .stage(
                &canonical.api_key,
                |api_key| search_settings_from_request(request, api_key),
                settings_validate::stored_search_settings,
            )
            .await?;

        let settings = self.db.save_search_settings(&candidate).await?;
        let has = self.search_secrets.is_present().await?;
        Ok(search_response_from_stored(settings, has))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        mappers::{
            config_from_stored, docling_settings_from_request, response_from_stored,
            search_response_from_stored, search_settings_from_request,
        },
        runtime_mappers::{runtime_settings_from_request, runtime_settings_response},
        validate::{
            docling_request as validate_docling_request,
            runtime_settings_request as validate_runtime_settings_request,
            search_request as validate_search_request,
            stored_search_settings as validate_stored_search_settings,
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
                max_inflight: context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
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
            max_inflight: context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
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
                max_concurrency: 1,
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

        let response = response_from_stored(
            DoclingSettingsSource::Database,
            true,
            settings.clone(),
            settings.api_key.is_some(),
        );
        assert_eq!(
            response.vlm.openai_base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(response.vlm.has_api_key);
        // Presence is an input, not a fact about the value, so a response can
        // report a key it is deliberately not holding open.
        assert!(
            !response_from_stored(DoclingSettingsSource::Database, true, settings, false)
                .vlm
                .has_api_key
        );
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

        let stored = runtime_settings_from_request(&request);
        assert!(stored.file_library.trusted_proxy_enabled);
        assert!(
            runtime_settings_response(stored, false, false)
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
    fn the_s3_secret_key_is_never_persisted_and_is_reported_from_presence() {
        let mut request = sample_runtime_request();
        request.file_library.s3 = Some(crate::contracts::UpdateRuntimeS3Settings {
            endpoint: "https://objects.internal".to_string(),
            region: "internal".to_string(),
            bucket: "library".to_string(),
            prefix: "/staging/".to_string(),
            path_style: true,
            access_key: " AKIA ".to_string(),
            secret_key: None,
        });

        // The access key and the rest of the block are stored settings; the secret
        // key is not one of them, so the mapper leaves it unresolved and the store
        // remains its only representation.
        let stored = runtime_settings_from_request(&request);
        let s3 = stored
            .file_library
            .s3
            .clone()
            .expect("requested s3 settings are stored");
        assert_eq!(
            s3.secret_key, None,
            "the settings row must not carry the secret key"
        );
        assert_eq!(stored.embedding.api_key, None);
        assert_eq!(s3.access_key, "AKIA", "the access key is not a secret");
        assert_eq!(s3.prefix, "staging");

        // Presence is an input, not a fact about the value: a response can report
        // a sealed key it is deliberately not holding open.
        let response = runtime_settings_response(stored.clone(), false, true);
        let response_s3 = response.file_library.s3.expect("s3 settings are projected");
        assert!(response_s3.has_secret_key);
        assert_eq!(response_s3.access_key, "AKIA");
        let without_s3 = runtime_settings_response(stored, false, false);
        assert!(
            !without_s3
                .file_library
                .s3
                .expect("s3 settings are projected")
                .has_secret_key
        );
    }

    #[test]
    fn search_response_hides_api_key() {
        let settings = default_search_settings();

        // `has_api_key` is an input, never a fact about the stored value: the
        // response can report a sealed key it is deliberately not holding open.
        let response = search_response_from_stored(settings.clone(), true);
        assert!(response.has_api_key);
        assert!(!search_response_from_stored(settings, false).has_api_key);
    }

    fn sample_search_request() -> UpdateSearchSettingsRequest {
        UpdateSearchSettingsRequest {
            mode: crate::contracts::SearchMode::Hybrid,
            rerank_enabled: true,
            rerank_base_url: "https://openrouter.ai/api/v1".to_string(),
            rerank_model: "cohere/rerank-4-fast".to_string(),
            candidate_limit: 40,
            timeout_secs: 10,
            api_key: None,
            clear_api_key: false,
            vector_weight: context69_contracts::settings::SEARCH_VECTOR_WEIGHT_DEFAULT,
            keyword_weight: context69_contracts::settings::SEARCH_KEYWORD_WEIGHT_DEFAULT,
        }
    }

    #[test]
    fn search_request_rejects_empty_rerank_model() {
        let mut request = sample_search_request();
        request.rerank_model = " ".to_string();

        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("rerank_model"));
    }

    #[test]
    fn search_request_rejects_out_of_range_fusion_weights() {
        let mut request = sample_search_request();
        request.vector_weight = 1.5;
        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("vector_weight"));

        let mut request = sample_search_request();
        request.keyword_weight = -0.1;
        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("keyword_weight"));
    }

    #[test]
    fn search_request_rejects_weights_above_unit_budget() {
        let mut request = sample_search_request();
        request.vector_weight = 0.7;
        request.keyword_weight = 0.4;
        let error = validate_search_request(&request).expect_err("request should be invalid");
        assert!(error.to_string().contains("must not sum above 1"));
    }

    #[test]
    fn search_request_accepts_default_and_partial_fusion_weights() {
        validate_search_request(&sample_search_request()).expect("default weights are valid");
        let mut request = sample_search_request();
        request.vector_weight = 0.4;
        request.keyword_weight = 0.3;
        validate_search_request(&request).expect("weights under the unit budget are valid");
    }

    #[test]
    fn legacy_search_request_without_weights_defaults_to_current_blend() {
        let payload = serde_json::json!({
            "mode": "hybrid",
            "rerank_enabled": true,
            "rerank_base_url": "https://openrouter.ai/api/v1",
            "rerank_model": "cohere/rerank-4-fast",
            "candidate_limit": 40,
            "timeout_secs": 10
        });
        let request: UpdateSearchSettingsRequest =
            serde_json::from_value(payload).expect("legacy request without fusion weights");
        assert_eq!(request.vector_weight, 0.55);
        assert_eq!(request.keyword_weight, 0.35);
        validate_search_request(&request).expect("defaulted weights should be valid");
    }

    #[test]
    fn search_fusion_weights_round_trip_through_mappers_and_stored_validation() {
        let mut request = sample_search_request();
        request.vector_weight = 0.6;
        request.keyword_weight = 0.2;

        let stored = search_settings_from_request(&request, None);
        assert_eq!(stored.vector_weight, 0.6);
        assert_eq!(stored.keyword_weight, 0.2);
        validate_stored_search_settings(&stored).expect("stored weights should be valid");

        let response = search_response_from_stored(stored.clone(), false);
        assert_eq!(response.vector_weight, 0.6);
        assert_eq!(response.keyword_weight, 0.2);

        let mut invalid = stored;
        invalid.keyword_weight = 0.5;
        let error =
            validate_stored_search_settings(&invalid).expect_err("stored weights should fail");
        assert!(error.to_string().contains("sum above 1"));
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

        let config = config_from_stored(stored.clone(), None);
        assert_eq!(
            config.vlm.picture_description_preset.as_deref(),
            Some("smolvlm"),
            "stored -> runtime config must surface picture_description_preset"
        );

        let response = response_from_stored(DoclingSettingsSource::Database, true, stored, false);
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

    #[test]
    fn validate_docling_vlm_shape_accepts_preset_only_configuration() {
        let mut settings = sample_stored();
        settings.picture_description_preset = Some("granite_vision".to_string());

        validate_docling_vlm_shape(&settings)
            .expect("preset-only VLM settings should be a valid stored configuration");
    }

    #[test]
    fn validate_docling_vlm_shape_rejects_preset_combined_with_partial_legacy_fields() {
        let mut settings = sample_stored();
        settings.picture_description_preset = Some("granite_vision".to_string());
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        // api_key deliberately missing to make the stored config unsafe
        // and exercise the explicit preset+legacy error message.

        let error = validate_docling_vlm_shape(&settings)
            .expect_err("preset combined with partial legacy fields must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("picture_description_preset"),
            "error must name picture_description_preset but was: {message}"
        );
        assert!(
            message.contains("legacy OpenAI VLM bundle"),
            "error must call out the legacy bundle but was: {message}"
        );
    }

    #[test]
    fn validate_docling_vlm_shape_rejects_preset_combined_with_complete_legacy_bundle() {
        let mut settings = sample_stored();
        settings.picture_description_preset = Some("granite_vision".to_string());
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        let error = validate_docling_vlm_shape(&settings)
            .expect_err("preset combined with the full legacy bundle must be rejected");
        assert!(error.to_string().contains("picture_description_preset"));
    }

    #[test]
    fn validate_docling_vlm_shape_keeps_complete_custom_vlm_valid() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        settings.picture_description_model = Some("gpt-4o-mini".to_string());
        settings.code_formula_model = Some("gpt-4o-mini".to_string());

        validate_docling_vlm_shape(&settings)
            .expect("complete custom VLM bundle must remain valid");
    }

    #[test]
    fn validate_docling_vlm_shape_rejects_partial_custom_vlm() {
        let mut settings = sample_stored();
        settings.openai_base_url = Some("https://openrouter.ai/api/v1".to_string());
        settings.api_key = Some("secret".to_string());
        settings.vlm_pipeline_model = Some("gemini".to_string());
        // picture_description_model + code_formula_model missing

        let error = validate_docling_vlm_shape(&settings)
            .expect_err("partial custom VLM bundle must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("fully configured together"),
            "error must call out the all-or-none model rule but was: {message}"
        );
    }

    #[test]
    fn docling_request_accepts_bounded_max_inflight() {
        let mut request = sample_request();
        for limit in [
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX,
        ] {
            request.connection.max_inflight = limit;
            validate_docling_request(&request).expect("bounded max_inflight should be valid");
        }
    }

    #[test]
    fn docling_request_rejects_out_of_range_max_inflight() {
        let mut request = sample_request();
        request.connection.max_inflight = 0;
        let error =
            validate_docling_request(&request).expect_err("zero max_inflight must be rejected");
        assert!(error.to_string().contains("max_inflight"));

        request.connection.max_inflight =
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX + 1;
        let error = validate_docling_request(&request)
            .expect_err("over-ceiling max_inflight must be rejected");
        assert!(error.to_string().contains("max_inflight"));
    }

    #[test]
    fn docling_mapper_round_trips_max_inflight() {
        let mut request = sample_request();
        request.connection.max_inflight = 3;
        let stored = docling_settings_from_request(&request, None);
        assert_eq!(stored.max_inflight, 3);

        let response =
            response_from_stored(DoclingSettingsSource::Database, true, stored.clone(), false);
        assert_eq!(response.connection.max_inflight, 3);

        let config = config_from_stored(stored, None);
        assert_eq!(config.connection.max_inflight, 3);
    }

    #[test]
    fn legacy_docling_request_without_max_inflight_defaults_to_single_worker() {
        let payload = serde_json::json!({
            "connection": {
                "base_url": "http://docling:5001",
                "timeout_secs": 120,
                "poll_interval_secs": 2,
                "task_timeout_secs": 3600
            },
            "vlm": {}
        });
        let request: UpdateDoclingSettingsRequest =
            serde_json::from_value(payload).expect("legacy request without max_inflight");
        assert_eq!(
            request.connection.max_inflight,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT,
            "legacy API payloads must default to the single-worker ceiling"
        );
        validate_docling_request(&request).expect("defaulted request should be valid");
    }
}
