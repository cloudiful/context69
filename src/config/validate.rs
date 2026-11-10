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
pub(super) fn validate_embedding_config(config: &EmbeddingConfig) -> Result<()> {
    validate_provider_config("embedding", config)
}

/// Validates the optional code embedding block when a deployment declares one.
///
/// An absent block is the disabled default. When present it uses the same
/// provider rules as the document block, without inheriting any document value.
pub(super) fn validate_code_embedding_config(config: Option<&EmbeddingConfig>) -> Result<()> {
    match config {
        Some(config) => validate_provider_config("code_embedding", config),
        None => Ok(()),
    }
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

    use super::validate_code_embedding_config;

    fn code_config() -> super::EmbeddingConfig {
        super::EmbeddingConfig {
            base_url: "http://code.invalid/v1".to_string(),
            api_key: None,
            model: "code-model".to_string(),
            query_model: None,
            dimensions: 1024,
            timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn a_complete_code_embedding_config_is_accepted() {
        assert!(validate_code_embedding_config(Some(&code_config())).is_ok());
    }
}
