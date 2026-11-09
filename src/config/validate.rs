use std::collections::HashSet;

use anyhow::{Result, anyhow};

use context69_secret_crypto::MasterKey;

use crate::{
    chunking::ChunkBudgetMode,
    docling::{DoclingConfig, resolve_vlm_runtime_config},
};

use super::types::{
    AppConfig, AuthConfig, ChunkingFileConfig, ConnectionConfig, EmbeddingConfig,
    FileLibraryConfig, SchedulerConfig, SecretStoreConfig, SourceConfig,
};

/// Validates the embedding configuration of a deployment.
///
/// The batching limits are stored in a PostgreSQL BIGINT, so a value the column
/// cannot hold is refused here rather than at the first settings save.
pub(super) fn validate_embedding_config(config: &EmbeddingConfig) -> Result<()> {
    validate_provider_config("embedding", config)
}

/// Validates the optional code embedding block when a deployment declares one.
///
/// An absent block is the disabled default. When present it uses the same
/// provider rules as the document block, without inheriting any document value.
pub(super) fn validate_code_embedding_config(config: Option<&EmbeddingConfig>) -> Result<()> {
    match config {
        Some(config) => {
            validate_provider_config("code_embedding", config)?;
            reject_non_default_code_input_policy(config)
        }
        None => Ok(()),
    }
}

/// Keeps the P1 fixed code input policy truthful.
///
/// Code chunks are embedded exactly as chunked; a custom token cap, server
/// prefix, or special-token switch would silently change the vector space while
/// the published fingerprint kept claiming the fixed policy.
fn reject_non_default_code_input_policy(config: &EmbeddingConfig) -> Result<()> {
    if config.input_policy.is_default() {
        return Ok(());
    }
    Err(anyhow!(
        "code_embedding.input_policy must remain the fixed default code input policy"
    ))
}

fn validate_provider_config(section: &str, config: &EmbeddingConfig) -> Result<()> {
    if config.base_url.trim().is_empty() {
        return Err(anyhow!("{section}.base_url must not be empty"));
    }
    if config.model.trim().is_empty() {
        return Err(anyhow!("{section}.model must not be empty"));
    }
    if config.dimensions == 0 {
        return Err(anyhow!("{section}.dimensions must be greater than 0"));
    }
    if config.timeout.as_secs() == 0 {
        return Err(anyhow!("{section}.timeout_secs must be greater than 0"));
    }
    let limits =
        crate::embedding::embedding_batch::EmbeddingBatchLimits::from_settings(&config.batching);
    if !limits.is_positive() {
        return Err(anyhow!(
            "{section}.batching limits must all be greater than 0"
        ));
    }
    config
        .input_policy
        .validate()
        .map_err(|reason| anyhow!("{section}.input_policy is invalid: {reason}"))?;
    for (name, value) in [
        (
            "max_items_per_request",
            config.batching.max_items_per_request,
        ),
        ("max_item_chars", config.batching.max_item_chars),
        ("max_request_chars", config.batching.max_request_chars),
        ("max_request_bytes", config.batching.max_request_bytes),
    ] {
        if i64::try_from(value).is_err() {
            return Err(anyhow!(
                "{section}.batching.{name} must be at most {}",
                i64::MAX
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_scheduler_config(config: &SchedulerConfig) -> Result<()> {
    if config.max_concurrency == 0 {
        return Err(anyhow!("scheduler.max_concurrency must be greater than 0"));
    }
    if config.execution_guard_ttl.as_secs() == 0 {
        return Err(anyhow!(
            "scheduler.execution_guard_ttl_secs must be greater than 0"
        ));
    }
    if config.execution_guard_renew_interval.as_secs() == 0 {
        return Err(anyhow!(
            "scheduler.execution_guard_renew_interval_secs must be greater than 0"
        ));
    }
    if config.execution_guard_renew_interval >= config.execution_guard_ttl {
        return Err(anyhow!(
            "scheduler.execution_guard_renew_interval_secs must be less than scheduler.execution_guard_ttl_secs"
        ));
    }
    Ok(())
}

pub(super) fn validate_docling_config(config: Option<&DoclingConfig>) -> Result<()> {
    let Some(docling) = config else {
        return Ok(());
    };

    if docling.connection.base_url.trim().is_empty() {
        return Err(anyhow!("docling.base_url must not be empty"));
    }
    if docling.connection.timeout.as_secs() == 0 {
        return Err(anyhow!("docling.timeout_secs must be greater than 0"));
    }
    if docling.connection.poll_interval.as_secs() == 0 {
        return Err(anyhow!("docling.poll_interval_secs must be greater than 0"));
    }
    resolve_vlm_runtime_config(&docling.vlm)?;
    Ok(())
}

pub(super) fn validate_storage_config(
    file_library: &FileLibraryConfig,
    chunking: &ChunkingFileConfig,
) -> Result<()> {
    if file_library.max_upload_size_mb == 0 {
        return Err(anyhow!(
            "file_library.max_upload_size_mb must be greater than 0"
        ));
    }
    if file_library.max_upload_request_size_mb == 0 {
        return Err(anyhow!(
            "file_library.max_upload_request_size_mb must be greater than 0"
        ));
    }
    if file_library.max_upload_request_size_mb < file_library.max_upload_size_mb {
        return Err(anyhow!(
            "file_library.max_upload_request_size_mb must be greater than or equal to file_library.max_upload_size_mb"
        ));
    }
    if file_library.ingest_concurrency == 0 {
        return Err(anyhow!(
            "file_library.ingest_concurrency must be greater than 0"
        ));
    }
    if file_library.url_import_concurrency == 0 {
        return Err(anyhow!(
            "file_library.url_import_concurrency must be greater than 0"
        ));
    }
    if file_library.url_import_min_interval_ms == 0 {
        return Err(anyhow!(
            "file_library.url_import_min_interval_ms must be greater than 0"
        ));
    }
    if chunking.budget.overlap_chars >= chunking.budget.max_chars {
        return Err(anyhow!(
            "chunking.overlap_chars must be smaller than chunking.max_chars"
        ));
    }
    if let Some(sha256) = &chunking.budget.tokenizer_sha256
        && !crate::chunking::chunking_token_artifact::is_lower_sha256(sha256)
    {
        return Err(anyhow!(
            "chunking.tokenizer_sha256 must be a lowercase 64-hex artifact hash"
        ));
    }
    if chunking
        .budget
        .tokenizer_artifact_dir
        .as_os_str()
        .is_empty()
    {
        return Err(anyhow!("chunking.tokenizer_artifact_dir must not be empty"));
    }
    if chunking.tokenizer_download_dir.as_os_str().is_empty() {
        return Err(anyhow!("chunking.tokenizer_download_dir must not be empty"));
    }
    if chunking.budget.unit == ChunkBudgetMode::Tokens {
        if chunking.budget.max_tokens == 0 {
            return Err(anyhow!("chunking.max_tokens must be greater than 0"));
        }
        if chunking.budget.overlap_tokens >= chunking.budget.max_tokens {
            return Err(anyhow!(
                "chunking.overlap_tokens must be smaller than chunking.max_tokens"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_sources_config(
    connections: &[ConnectionConfig],
    sources: &[SourceConfig],
) -> Result<()> {
    const MAX_SOURCE_EXAMPLE_QUERIES: usize = 6;
    const MAX_SOURCE_EXAMPLE_QUERY_LEN: usize = 120;

    let connection_names = connections
        .iter()
        .map(|connection| connection.name.as_str())
        .collect::<HashSet<_>>();

    for source in sources {
        if !connection_names.contains(source.connection.as_str()) {
            return Err(anyhow!(
                "source {} references unknown connection {}",
                source.key,
                source.connection
            ));
        }
        if source.connector.base_query.trim().is_empty() {
            return Err(anyhow!(
                "source {} base_query must not be empty",
                source.key
            ));
        }
        if source.connector.batch_size <= 0 {
            return Err(anyhow!(
                "source {} batch_size must be greater than 0",
                source.key
            ));
        }
        if source.example_queries.len() > MAX_SOURCE_EXAMPLE_QUERIES {
            return Err(anyhow!(
                "source {} example_queries must contain at most {} items",
                source.key,
                MAX_SOURCE_EXAMPLE_QUERIES
            ));
        }
        if source
            .example_queries
            .iter()
            .any(|query| query.chars().count() > MAX_SOURCE_EXAMPLE_QUERY_LEN)
        {
            return Err(anyhow!(
                "source {} example_queries items must be at most {} characters",
                source.key,
                MAX_SOURCE_EXAMPLE_QUERY_LEN
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_auth_config(config: &AuthConfig) -> Result<()> {
    if config
        .session_valkey_url
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(anyhow!(
            "auth.session_valkey_url must not be empty when set"
        ));
    }
    if config
        .session_secret_key
        .as_deref()
        .is_some_and(|value| value.trim().len() < 32)
    {
        return Err(anyhow!(
            "auth.session_secret_key must be at least 32 characters when set"
        ));
    }
    if config.session_idle_ttl.as_secs() == 0 {
        return Err(anyhow!("auth.session_idle_ttl_secs must be greater than 0"));
    }
    if let Some(admin) = &config.bootstrap_admin
        && admin.login_name.trim().is_empty()
    {
        return Err(anyhow!("auth.bootstrap_admin.login_name must not be empty"));
    }
    if let Some(admin) = &config.bootstrap_admin
        && admin.display_name.trim().is_empty()
    {
        return Err(anyhow!(
            "auth.bootstrap_admin.display_name must not be empty"
        ));
    }
    if let Some(admin) = &config.bootstrap_admin
        && admin.password.trim().len() < 8
    {
        return Err(anyhow!(
            "auth.bootstrap_admin.password must be at least 8 characters"
        ));
    }
    Ok(())
}

pub(super) fn validate_app_config(config: &AppConfig) -> Result<()> {
    let Some(master_secret) = config
        .master_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    // Parsing proves the deployment value really is a 32-byte key. The error
    // never carries the rejected value, so this is safe to surface.
    MasterKey::from_base64(master_secret)
        .map_err(|_| anyhow!("app.master_secret must be a base64-encoded 32-byte key"))?;
    Ok(())
}

pub(super) fn validate_secret_store_config(config: &SecretStoreConfig) -> Result<()> {
    if config.key_version == 0 {
        return Err(anyhow!("secret_store.key_version must be greater than 0"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use context69_contracts::settings::EmbeddingBatchingSettings;

    use super::validate_code_embedding_config;

    fn code_config() -> super::EmbeddingConfig {
        super::EmbeddingConfig {
            base_url: "http://code.invalid/v1".to_string(),
            api_key: None,
            model: "code-model".to_string(),
            query_model: None,
            dimensions: 1024,
            timeout: Duration::from_secs(30),
            batching: EmbeddingBatchingSettings::default(),
            input_policy: Default::default(),
        }
    }

    #[test]
    fn a_custom_code_embedding_input_policy_is_rejected_before_installation() {
        let mut config = code_config();
        assert!(
            validate_code_embedding_config(Some(&config)).is_ok(),
            "the fixed default policy is the only accepted code policy"
        );

        config.input_policy.max_input_tokens = Some(256);
        let error = validate_code_embedding_config(Some(&config))
            .expect_err("a custom token cap must not reach the provider");
        assert!(error.to_string().contains("input_policy"), "{error}");
    }
}
