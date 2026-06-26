use ::config::{ReadOptions, read};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{
    defaults::{APP_NAME, CONFIG_ENV_PREFIX},
    types::{AppDbConfig, Config, FileConfig},
    validate::{
        validate_auth_config, validate_docling_config, validate_scheduler_config,
        validate_sources_config, validate_storage_config,
    },
};

pub(super) fn load_config() -> Result<Config> {
    let file_config: FileConfig = read(
        APP_NAME,
        Some(ReadOptions::with_env_prefix(CONFIG_ENV_PREFIX)),
    )
    .context("failed to load config")?;
    file_config.try_into()
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct AppDbOnlyConfig {
    app_db: Option<AppDbConfig>,
}

pub fn load_app_db_url() -> Result<Option<String>> {
    let config: AppDbOnlyConfig = read(
        APP_NAME,
        Some(ReadOptions::with_env_prefix(CONFIG_ENV_PREFIX)),
    )
    .context("failed to load config")?;

    Ok(config
        .app_db
        .and_then(|app_db| sanitize_optional_string(Some(app_db.url))))
}

pub(super) fn validate_loaded_config(config: &FileConfig) -> Result<()> {
    if config.api.bind_addr.trim().is_empty() {
        return Err(anyhow!("api.bind_addr must not be empty"));
    }
    if config.mcp.bind_addr.trim().is_empty() {
        return Err(anyhow!("mcp.bind_addr must not be empty"));
    }
    validate_scheduler_config(&config.scheduler)?;
    validate_docling_config(config.docling.as_ref())?;
    validate_auth_config(&config.auth)?;
    validate_storage_config(&config.file_library, &config.chunking)?;
    validate_sources_config(&config.connections, &config.sources)?;
    Ok(())
}

pub fn validate_legacy_runtime_import_config(config: &Config) -> Result<()> {
    if config.connections.is_empty() {
        return Err(anyhow!(
            "runtime settings are not initialized in database; define at least one source connection in config before first startup"
        ));
    }
    Ok(())
}

fn sanitize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
