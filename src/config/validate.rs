use std::collections::HashSet;

use anyhow::{Result, anyhow};

use crate::{
    chunking::ChunkingConfig,
    docling::{DoclingConfig, resolve_vlm_runtime_config},
};

use super::types::{
    AuthConfig, ConnectionConfig, FileLibraryConfig, SchedulerConfig, SourceConfig,
};

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
    chunking: &ChunkingConfig,
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
    if file_library.pdf_pages_per_task == 0 {
        return Err(anyhow!(
            "file_library.pdf_pages_per_task must be greater than 0"
        ));
    }
    if chunking.overlap_chars >= chunking.max_chars {
        return Err(anyhow!(
            "chunking.overlap_chars must be smaller than chunking.max_chars"
        ));
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
    if config.issuer.trim().is_empty() {
        return Err(anyhow!("auth.issuer must not be empty"));
    }
    if config.access_token_ttl.as_secs() == 0 {
        return Err(anyhow!("auth.access_token_ttl_secs must be greater than 0"));
    }
    if config.refresh_token_ttl.as_secs() == 0 {
        return Err(anyhow!(
            "auth.refresh_token_ttl_secs must be greater than 0"
        ));
    }
    if config.refresh_cookie_name.trim().is_empty() {
        return Err(anyhow!("auth.refresh_cookie_name must not be empty"));
    }
    if config.active_kid.trim().is_empty() {
        return Err(anyhow!("auth.active_kid must not be empty"));
    }
    if config.signing_keys.is_empty() {
        return Err(anyhow!("auth.signing_keys must not be empty"));
    }
    let mut found_active = false;
    for key in &config.signing_keys {
        if key.kid.trim().is_empty() {
            return Err(anyhow!("auth.signing_keys[].kid must not be empty"));
        }
        if key.secret.trim().is_empty() {
            return Err(anyhow!("auth.signing_keys[].secret must not be empty"));
        }
        if key.kid == config.active_kid {
            found_active = true;
        }
    }
    if !found_active {
        return Err(anyhow!(
            "auth.active_kid must match one of auth.signing_keys[].kid"
        ));
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
