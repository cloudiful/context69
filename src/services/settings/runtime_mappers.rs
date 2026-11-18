use crate::{
    config::{EmbeddingConfig, ImageEmbeddingConfig},
    contracts::settings::{RuntimeImageEmbeddingSettings, UpdateImageEmbeddingSettings},
    contracts::{
        RuntimeChunkingSettings, RuntimeCodeEmbeddingSettings, RuntimeEmbeddingSettings,
        RuntimeFileLibrarySettings, RuntimeQdrantSettings, RuntimeSchedulerSettings,
        RuntimeSettingsResponse, SecretPatch, UpdateCodeEmbeddingSettings, UpdateRuntimeS3Settings,
        UpdateRuntimeSettingsRequest,
    },
    db::{
        StoredRuntimeChunkingSettings, StoredRuntimeCodeEmbeddingSettings,
        StoredRuntimeEmbeddingSettings, StoredRuntimeFileLibrarySettings,
        StoredRuntimeImageEmbeddingSettings, StoredRuntimeQdrantSettings,
        StoredRuntimeSchedulerSettings, StoredRuntimeSettings,
    },
    support::normalize::normalize_optional_string,
};

use context69_contracts::settings::{
    CHUNKING_MAX_TOKENS_DEFAULT, CHUNKING_OVERLAP_TOKENS_DEFAULT, ChunkingTokenizer, ChunkingUnit,
};

use crate::chunking::chunking_token::TokenizerRef;
use crate::chunking::{ChunkBudget, ChunkBudgetMode, ChunkingConfig, TokenBudget};

use super::secrets::optional_key_patch;

/// The chunking budget of the runtime settings.
///
/// Both budgets are carried, so switching the unit never discards the values
/// of the other one.
pub(crate) fn chunk_budget(settings: &RuntimeChunkingSettings) -> ChunkBudget {
    ChunkBudget {
        mode: chunk_budget_mode(&settings.unit),
        chars: ChunkingConfig {
            max_chars: settings.max_chars,
            overlap_chars: settings.overlap_chars,
        },
        tokens: Some(TokenBudget {
            tokenizer: chunking_tokenizer(settings),
            max_tokens: settings.max_tokens,
            overlap_tokens: settings.overlap_tokens,
        }),
    }
}

/// The runtime settings of a chunking budget.
pub(crate) fn chunking_settings(budget: &ChunkBudget) -> RuntimeChunkingSettings {
    let tokens = budget.tokens.clone().unwrap_or(TokenBudget {
        tokenizer: TokenizerRef::Cl100kBase,
        max_tokens: CHUNKING_MAX_TOKENS_DEFAULT,
        overlap_tokens: CHUNKING_OVERLAP_TOKENS_DEFAULT,
    });
    let (tokenizer, tokenizer_sha256) = stored_chunking_tokenizer(&tokens.tokenizer);
    RuntimeChunkingSettings {
        max_chars: budget.chars.max_chars,
        overlap_chars: budget.chars.overlap_chars,
        unit: chunking_unit(budget.mode),
        tokenizer,
        tokenizer_sha256,
        max_tokens: tokens.max_tokens,
        overlap_tokens: tokens.overlap_tokens,
    }
}

/// The chunking budget of the stored settings.
pub(crate) fn stored_chunk_budget(settings: &StoredRuntimeChunkingSettings) -> ChunkBudget {
    settings.chunk_budget()
}

/// The stored chunking settings of a chunking budget.
fn stored_chunking_settings(budget: &ChunkBudget) -> StoredRuntimeChunkingSettings {
    let tokens = budget.tokens.clone().unwrap_or(TokenBudget {
        tokenizer: TokenizerRef::Cl100kBase,
        max_tokens: CHUNKING_MAX_TOKENS_DEFAULT,
        overlap_tokens: CHUNKING_OVERLAP_TOKENS_DEFAULT,
    });
    StoredRuntimeChunkingSettings {
        max_chars: budget.chars.max_chars,
        overlap_chars: budget.chars.overlap_chars,
        unit: budget.mode,
        tokenizer: tokens.tokenizer,
        max_tokens: tokens.max_tokens,
        overlap_tokens: tokens.overlap_tokens,
    }
}

/// The chunking unit of the request settings.
fn chunk_budget_mode(unit: &ChunkingUnit) -> ChunkBudgetMode {
    match unit {
        ChunkingUnit::Chars => ChunkBudgetMode::Chars,
        ChunkingUnit::Tokens => ChunkBudgetMode::Tokens.fresh(),
    }
}

/// The chunking unit of the stored settings.
fn chunking_unit(unit: ChunkBudgetMode) -> ChunkingUnit {
    match unit {
        ChunkBudgetMode::Chars => ChunkingUnit::Chars,
        ChunkBudgetMode::Tokens | ChunkBudgetMode::TokensV2 => ChunkingUnit::Tokens,
    }
}

/// The tokenizer of the request settings.
fn chunking_tokenizer(settings: &RuntimeChunkingSettings) -> TokenizerRef {
    match &settings.tokenizer {
        ChunkingTokenizer::Cl100kBase => TokenizerRef::Cl100kBase,
        ChunkingTokenizer::O200kBase => TokenizerRef::O200kBase,
        ChunkingTokenizer::HfLocal => TokenizerRef::HfLocal {
            sha256: settings.tokenizer_sha256.clone().unwrap_or_default(),
        },
    }
}

/// The tokenizer of the stored settings.
fn stored_chunking_tokenizer(tokenizer: &TokenizerRef) -> (ChunkingTokenizer, Option<String>) {
    match tokenizer {
        TokenizerRef::Cl100kBase => (ChunkingTokenizer::Cl100kBase, None),
        TokenizerRef::O200kBase => (ChunkingTokenizer::O200kBase, None),
        TokenizerRef::HfLocal { sha256 } => (ChunkingTokenizer::HfLocal, Some(sha256.clone())),
    }
}

/// The patch a runtime S3 update maps to.
///
/// A submitted secret key replaces the stored one and an absent or blank one
/// keeps it — a blank key has never been a clear. Dropping the S3 block drops the
/// credential with it, the same rule the Docling base URL follows, so a later
/// re-add cannot silently resurrect a credential the operator removed.
pub(super) fn s3_secret_patch(requested: Option<&UpdateRuntimeS3Settings>) -> SecretPatch {
    match requested {
        Some(s3) => optional_key_patch(s3.secret_key.clone()),
        None => SecretPatch::Clear,
    }
}

pub(super) fn runtime_settings_from_request(
    request: &UpdateRuntimeSettingsRequest,
) -> StoredRuntimeSettings {
    StoredRuntimeSettings {
        qdrant: StoredRuntimeQdrantSettings {
            url: request.qdrant.url.trim().to_string(),
            collection_name: request.qdrant.collection_name.trim().to_string(),
            recreate_on_dimension_mismatch: request.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: StoredRuntimeEmbeddingSettings {
            base_url: request.embedding.base_url.trim().to_string(),
            // The provider API key is not a runtime setting: the shared store owns
            // it, so the row never carries a plaintext copy of it.
            api_key: None,
            model: request.embedding.model.trim().to_string(),
            // A blank query model is the document model, so it is stored as no
            // query model at all.
            query_model: normalize_optional_string(request.embedding.query_model.clone()),
            dimensions: request.embedding.dimensions,
            timeout_secs: request.embedding.timeout_secs,
        },
        scheduler: StoredRuntimeSchedulerSettings {
            interval_secs: request.scheduler.interval_secs,
            run_on_start: request.scheduler.run_on_start,
            max_concurrency: request.scheduler.max_concurrency,
            job_id: request.scheduler.job_id.trim().to_string(),
            valkey_url: normalize_optional_string(request.scheduler.valkey_url.clone()),
        },
        chunking: stored_chunking_settings(&chunk_budget(&request.chunking)),
        file_library: StoredRuntimeFileLibrarySettings {
            storage_root: request.file_library.storage_root.trim().to_string(),
            max_upload_size_mb: request.file_library.max_upload_size_mb,
            trusted_proxy_enabled: request.file_library.trusted_proxy_enabled,
            // An absent cap keeps the stored one: a client that does not know
            // the setting never resets the policy.
            registry_max_ids_per_file: request.file_library.registry_max_ids_per_file,
            registry_max_ids_per_run: request.file_library.registry_max_ids_per_run,
            s3: request
                .file_library
                .s3
                .as_ref()
                .map(|s3| crate::db::StoredRuntimeS3Settings {
                    endpoint: s3.endpoint.trim().to_string(),
                    region: s3.region.trim().to_string(),
                    bucket: s3.bucket.trim().to_string(),
                    prefix: s3.prefix.trim_matches('/').to_string(),
                    path_style: s3.path_style,
                    access_key: s3.access_key.trim().to_string(),
                    // The secret key is not a runtime setting either: the shared
                    // store owns it, so the row keeps only the identifiers and
                    // whoever needs a usable configuration resolves the key.
                    secret_key: None,
                }),
        },
    }
}

/// The pure code stored-settings projection shared by settings apply, update,
/// and app hydration. The caller supplies the credential it resolved.
pub(crate) fn code_embedding_config(
    code: &StoredRuntimeCodeEmbeddingSettings,
    api_key: Option<String>,
) -> EmbeddingConfig {
    EmbeddingConfig {
        base_url: code.base_url.trim().to_string(),
        api_key,
        model: code.model.trim().to_string(),
        query_model: code.query_model.clone(),
        dimensions: code.dimensions,
        timeout: std::time::Duration::from_secs(code.timeout_secs),
    }
}

pub(super) fn stored_code_embedding_from_request(
    request: &UpdateCodeEmbeddingSettings,
) -> StoredRuntimeCodeEmbeddingSettings {
    StoredRuntimeCodeEmbeddingSettings {
        enabled: request.enabled,
        base_url: request.base_url.trim().to_string(),
        model: request.model.trim().to_string(),
        query_model: normalize_optional_string(request.query_model.clone()),
        dimensions: request.dimensions,
        timeout_secs: request.timeout_secs,
        // The provider API key is not a runtime setting: the shared store owns
        // it under its own purpose.
        api_key: None,
    }
}

/// The pure image stored-settings projection shared by settings apply, update,
/// and app hydration. The caller supplies the credential it resolved.
pub(crate) fn image_embedding_config(
    image: &StoredRuntimeImageEmbeddingSettings,
    api_key: Option<String>,
) -> ImageEmbeddingConfig {
    ImageEmbeddingConfig {
        base_url: image.base_url.trim().to_string(),
        api_key,
        model: image.model.trim().to_string(),
        query_model: image.query_model.clone(),
        dimensions: image.dimensions,
        timeout: std::time::Duration::from_secs(image.timeout_secs),
        collection_name: image.collection_name.trim().to_string(),
    }
}

pub(super) fn stored_image_embedding_from_request(
    request: &UpdateImageEmbeddingSettings,
) -> StoredRuntimeImageEmbeddingSettings {
    StoredRuntimeImageEmbeddingSettings {
        enabled: request.enabled,
        base_url: request.base_url.trim().to_string(),
        collection_name: request.collection_name.trim().to_string(),
        model: request.model.trim().to_string(),
        query_model: normalize_optional_string(request.query_model.clone()),
        dimensions: request.dimensions,
        timeout_secs: request.timeout_secs,
        // The provider API key is not a runtime setting: the shared store owns
        // it under its own purpose.
        api_key: None,
    }
}

/// `has_api_key` and `has_secret_key` are passed in rather than derived from the
/// stored values: a credential is answered from the shared store's metadata, so
/// presence has to be answerable without opening it.
#[cfg(test)]
pub(super) fn runtime_settings_response(
    settings: StoredRuntimeSettings,
    has_embedding_api_key: bool,
    has_s3_secret_key: bool,
    requires_rebuild: bool,
) -> RuntimeSettingsResponse {
    runtime_settings_response_with_code(
        settings,
        has_embedding_api_key,
        has_s3_secret_key,
        requires_rebuild,
        None,
        false,
    )
}

pub(super) fn runtime_settings_response_with_code(
    settings: StoredRuntimeSettings,
    has_embedding_api_key: bool,
    has_s3_secret_key: bool,
    requires_rebuild: bool,
    code: Option<StoredRuntimeCodeEmbeddingSettings>,
    has_code_api_key: bool,
) -> RuntimeSettingsResponse {
    let quota = settings.file_library.registry_quota();
    RuntimeSettingsResponse {
        qdrant: RuntimeQdrantSettings {
            url: settings.qdrant.url,
            collection_name: settings.qdrant.collection_name,
            recreate_on_dimension_mismatch: settings.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: RuntimeEmbeddingSettings {
            base_url: settings.embedding.base_url,
            model: settings.embedding.model,
            query_model: settings.embedding.query_model,
            dimensions: settings.embedding.dimensions,
            timeout_secs: settings.embedding.timeout_secs,
            has_api_key: has_embedding_api_key,
        },
        code_embedding: code.map(|code| RuntimeCodeEmbeddingSettings {
            enabled: code.enabled,
            base_url: code.base_url,
            model: code.model,
            query_model: code.query_model,
            dimensions: code.dimensions,
            timeout_secs: code.timeout_secs,
            has_api_key: has_code_api_key,
        }),
        image_embedding: None,
        scheduler: RuntimeSchedulerSettings {
            interval_secs: settings.scheduler.interval_secs,
            run_on_start: settings.scheduler.run_on_start,
            max_concurrency: settings.scheduler.max_concurrency,
            job_id: settings.scheduler.job_id,
            valkey_url: settings.scheduler.valkey_url,
        },
        chunking: chunking_settings(&stored_chunk_budget(&settings.chunking)),
        file_library: RuntimeFileLibrarySettings {
            storage_root: settings.file_library.storage_root,
            max_upload_size_mb: settings.file_library.max_upload_size_mb,
            trusted_proxy_enabled: settings.file_library.trusted_proxy_enabled,
            // The response reports the policy that is in effect, not the one
            // the request asked for.
            registry_max_ids_per_file: quota.max_ids_per_file,
            registry_max_ids_per_run: quota.max_ids_per_run,
            s3: settings
                .file_library
                .s3
                .map(|s3| crate::contracts::RuntimeS3SettingsResponse {
                    endpoint: s3.endpoint,
                    region: s3.region,
                    bucket: s3.bucket,
                    prefix: s3.prefix,
                    path_style: s3.path_style,
                    access_key: s3.access_key,
                    has_secret_key: has_s3_secret_key,
                }),
        },
        tokenizer_artifacts:
            crate::chunking::chunking_token_artifact::TokenizerCatalog::installed_or_empty()
                .descriptors()
                .to_vec(),
        tokenizer_presets: crate::services::settings::tokenizer_provisioning::preset_descriptors(),
        requires_rebuild,
    }
}

/// Attaches the optional image block to a response.
///
/// The image block is projected separately so the shared response builder keeps
/// its existing argument list; presence is still metadata-only.
pub(super) fn with_image_embedding(
    mut response: RuntimeSettingsResponse,
    image: Option<StoredRuntimeImageEmbeddingSettings>,
    has_image_api_key: bool,
) -> RuntimeSettingsResponse {
    response.image_embedding = image.map(|image| RuntimeImageEmbeddingSettings {
        enabled: image.enabled,
        base_url: image.base_url,
        model: image.model,
        query_model: image.query_model,
        dimensions: image.dimensions,
        timeout_secs: image.timeout_secs,
        collection_name: image.collection_name,
        has_api_key: has_image_api_key,
    });
    response
}

pub(super) fn default_runtime_settings_response() -> RuntimeSettingsResponse {
    let defaults = crate::config::Config::default();
    RuntimeSettingsResponse {
        qdrant: RuntimeQdrantSettings {
            url: defaults.qdrant.url,
            collection_name: defaults.qdrant.collection_name,
            recreate_on_dimension_mismatch: defaults.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: RuntimeEmbeddingSettings {
            base_url: defaults.embedding.base_url,
            model: defaults.embedding.model,
            query_model: defaults.embedding.query_model,
            dimensions: defaults.embedding.dimensions,
            timeout_secs: defaults.embedding.timeout.as_secs(),
            has_api_key: defaults
                .embedding
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
        },
        code_embedding: None,
        image_embedding: None,
        scheduler: RuntimeSchedulerSettings {
            interval_secs: defaults.scheduler.interval.as_secs(),
            run_on_start: defaults.scheduler.run_on_start,
            max_concurrency: defaults.scheduler.max_concurrency,
            job_id: defaults.scheduler.job_id,
            valkey_url: defaults.scheduler.valkey_url,
        },
        chunking: chunking_settings(&defaults.chunking.chunk_budget()),
        file_library: RuntimeFileLibrarySettings {
            storage_root: defaults.file_library.storage_root.display().to_string(),
            max_upload_size_mb: defaults.file_library.max_upload_size_mb,
            trusted_proxy_enabled: defaults.file_library.trusted_proxy_enabled,
            registry_max_ids_per_file: defaults.registry_quota.registry_max_ids_per_file,
            registry_max_ids_per_run: defaults.registry_quota.registry_max_ids_per_run,
            s3: defaults
                .file_library
                .s3
                .map(|s3| crate::contracts::RuntimeS3SettingsResponse {
                    endpoint: s3.endpoint,
                    region: s3.region,
                    bucket: s3.bucket,
                    prefix: s3.prefix,
                    path_style: s3.path_style,
                    access_key: s3.access_key,
                    // No runtime settings row means no store row: the S3 secret
                    // key is only ever written together with that row, so the
                    // deployment default is the whole truth here.
                    has_secret_key: !s3.secret_key.is_empty(),
                }),
        },
        tokenizer_artifacts:
            crate::chunking::chunking_token_artifact::TokenizerCatalog::installed_or_empty()
                .descriptors()
                .to_vec(),
        tokenizer_presets: crate::services::settings::tokenizer_provisioning::preset_descriptors(),
        requires_rebuild: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        image_embedding_config, s3_secret_patch, stored_image_embedding_from_request,
        with_image_embedding,
    };
    use crate::contracts::settings::UpdateImageEmbeddingSettings;
    use crate::contracts::{RuntimeSettingsResponse, SecretPatch, UpdateRuntimeS3Settings};
    use crate::db::StoredRuntimeImageEmbeddingSettings;
    fn s3_request(secret_key: Option<&str>) -> UpdateRuntimeS3Settings {
        UpdateRuntimeS3Settings {
            endpoint: "https://objects.invalid".to_string(),
            region: "internal".to_string(),
            bucket: "library".to_string(),
            prefix: String::new(),
            path_style: false,
            access_key: "AKIA-EXAMPLE".to_string(),
            secret_key: secret_key.map(str::to_string),
        }
    }

    #[test]
    fn an_s3_update_maps_to_set_keep_or_clear() {
        assert_eq!(
            s3_secret_patch(Some(&s3_request(Some("new")))),
            SecretPatch::Set("new".to_string())
        );
        // An absent or blank key keeps what is stored; a blank key has never been
        // a way to clear it.
        assert_eq!(s3_secret_patch(Some(&s3_request(None))), SecretPatch::Keep);
        assert_eq!(
            s3_secret_patch(Some(&s3_request(Some("   ")))),
            SecretPatch::Keep
        );
        // Dropping the S3 block drops the credential with it, so a re-add cannot
        // resurrect one the operator removed.
        assert_eq!(s3_secret_patch(None), SecretPatch::Clear);
    }

    #[test]
    fn a_fresh_token_budget_carries_the_bounded_cut_and_keeps_the_public_unit() {
        use super::{chunk_budget, chunking_settings, stored_chunking_settings};
        use crate::chunking::{ChunkBudget, ChunkBudgetMode};
        use context69_contracts::settings::{
            ChunkingTokenizer, ChunkingUnit, RuntimeChunkingSettings,
        };

        let request = RuntimeChunkingSettings {
            max_chars: 1200,
            overlap_chars: 200,
            unit: ChunkingUnit::Tokens,
            tokenizer: ChunkingTokenizer::O200kBase,
            tokenizer_sha256: None,
            max_tokens: 512,
            overlap_tokens: 64,
        };
        let budget = chunk_budget(&request);
        assert_eq!(budget.mode, ChunkBudgetMode::TokensV2);
        assert_eq!(budget.identity(), "tokens_v2:o200k_base:512:64");

        let response = chunking_settings(&budget);
        assert_eq!(
            response.unit,
            ChunkingUnit::Tokens,
            "the public unit never exposes the stored token version"
        );

        let stored = stored_chunking_settings(&budget);
        assert_eq!(
            stored.chunk_budget().identity(),
            budget.identity(),
            "a stored settings row keeps the bounded version"
        );

        let legacy = ChunkBudget::tokens(
            crate::chunking::chunking_token::TokenizerKind::Cl100kBase,
            512,
            64,
        );
        assert_eq!(chunking_settings(&legacy).unit, ChunkingUnit::Tokens);
        let stored = stored_chunking_settings(&legacy);
        assert_eq!(
            stored.unit,
            ChunkBudgetMode::Tokens,
            "the stored unit keeps its persisted name"
        );
        assert_eq!(
            stored.chunk_budget().mode,
            ChunkBudgetMode::TokensV2,
            "saved settings are a fresh producer, so the next task uses the bounded cut"
        );
    }

    #[test]
    fn an_image_request_round_trips_through_the_stored_projection() {
        let request = UpdateImageEmbeddingSettings {
            enabled: true,
            base_url: " https://images.invalid/v1 ".to_string(),
            model: " image-model ".to_string(),
            query_model: Some(" query-model ".to_string()),
            dimensions: 768,
            timeout_secs: 30,
            collection_name: " context69_images ".to_string(),
            api_key: SecretPatch::Keep,
        };
        let stored = stored_image_embedding_from_request(&request);
        assert_eq!(stored.base_url, "https://images.invalid/v1");
        assert_eq!(stored.collection_name, "context69_images");
        assert!(stored.api_key.is_none());
        let config = image_embedding_config(&stored, Some("image-key".to_string()));
        assert_eq!(config.model, "image-model");
        assert_eq!(config.query_model.as_deref(), Some("query-model"));
        assert_eq!(config.dimensions, 768);
        assert_eq!(config.collection_name, "context69_images");
        assert_eq!(config.api_key.as_deref(), Some("image-key"));
    }

    #[test]
    fn the_image_response_projects_presence_without_the_credential() {
        let stored = StoredRuntimeImageEmbeddingSettings {
            enabled: false,
            base_url: "https://images.invalid/v1".to_string(),
            collection_name: "context69_images".to_string(),
            model: "image-model".to_string(),
            query_model: None,
            dimensions: 768,
            timeout_secs: 30,
            api_key: None,
        };
        let response = with_image_embedding(
            RuntimeSettingsResponse {
                qdrant: crate::contracts::RuntimeQdrantSettings {
                    url: "http://qdrant:6334".to_string(),
                    collection_name: "context69".to_string(),
                    recreate_on_dimension_mismatch: false,
                },
                embedding: crate::contracts::RuntimeEmbeddingSettings {
                    base_url: "https://text.invalid/v1".to_string(),
                    model: "text-model".to_string(),
                    query_model: None,
                    dimensions: 768,
                    timeout_secs: 30,
                    has_api_key: false,
                },
                code_embedding: None,
                image_embedding: None,
                scheduler: crate::contracts::RuntimeSchedulerSettings {
                    interval_secs: 60,
                    run_on_start: false,
                    max_concurrency: 1,
                    job_id: "job".to_string(),
                    valkey_url: None,
                },
                chunking: crate::contracts::RuntimeChunkingSettings {
                    max_chars: 1200,
                    overlap_chars: 200,
                    unit: context69_contracts::settings::ChunkingUnit::Chars,
                    tokenizer: context69_contracts::settings::ChunkingTokenizer::Cl100kBase,
                    tokenizer_sha256: None,
                    max_tokens: 1024,
                    overlap_tokens: 128,
                },
                file_library: crate::contracts::RuntimeFileLibrarySettings {
                    storage_root: "/tmp/library".to_string(),
                    max_upload_size_mb: 16,
                    trusted_proxy_enabled: false,
                    registry_max_ids_per_file: 16,
                    registry_max_ids_per_run: 16,
                    s3: None,
                },
                tokenizer_artifacts: Vec::new(),
                tokenizer_presets: Vec::new(),
                requires_rebuild: false,
            },
            Some(stored),
            true,
        );
        let image = response.image_embedding.expect("image block");
        assert!(!image.enabled);
        assert!(image.has_api_key);
        assert_eq!(image.collection_name, "context69_images");
    }
}
