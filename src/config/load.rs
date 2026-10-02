use ::config::{ReadOptions, read};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{
    defaults::{APP_NAME, CONFIG_ENV_PREFIX, DATABASE_URL_ENV_VAR},
    types::{AppDbConfig, Config, FileConfig},
    validate::{
        validate_app_config, validate_auth_config, validate_docling_config,
        validate_scheduler_config, validate_secret_store_config, validate_sources_config,
        validate_storage_config,
    },
};

// Retired app-database override. It is ignored: the generic
// `CONFIG_ENV_PREFIX` reader still merges it into `app_db.url`, so callers
// below restore the file value whenever it is present.
const RETIRED_APP_DB_URL_ENV_VAR: &str = "CONTEXT69_APP_DB__URL";

pub(super) fn load_config() -> Result<Config> {
    let mut file_config: FileConfig = read(
        APP_NAME,
        Some(ReadOptions::with_env_prefix(CONFIG_ENV_PREFIX)),
    )
    .context("failed to load config")?;
    restore_file_app_db_url(&mut file_config)?;
    if let Some(url) = database_url_from_env() {
        file_config.app_db.url = url;
    }
    file_config.try_into()
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct AppDbOnlyConfig {
    app_db: Option<AppDbConfig>,
}

pub fn load_app_db_url() -> Result<Option<String>> {
    let mut config: AppDbOnlyConfig = read(
        APP_NAME,
        Some(ReadOptions::with_env_prefix(CONFIG_ENV_PREFIX)),
    )
    .context("failed to load config")?;
    if std::env::var(RETIRED_APP_DB_URL_ENV_VAR).is_ok() {
        let base: AppDbOnlyConfig = read(APP_NAME, None).context("failed to load base config")?;
        config.app_db = base.app_db;
    }
    let configured = config
        .app_db
        .and_then(|app_db| sanitize_optional_string(Some(app_db.url)));
    Ok(resolve_app_db_url(database_url_from_env(), configured))
}

fn restore_file_app_db_url(file_config: &mut FileConfig) -> Result<()> {
    if std::env::var(RETIRED_APP_DB_URL_ENV_VAR).is_err() {
        return Ok(());
    }
    let base: FileConfig = read(APP_NAME, None).context("failed to load base config")?;
    file_config.app_db = base.app_db;
    Ok(())
}

fn database_url_from_env() -> Option<String> {
    sanitize_optional_string(std::env::var(DATABASE_URL_ENV_VAR).ok())
}

fn resolve_app_db_url(database_url: Option<String>, configured: Option<String>) -> Option<String> {
    let database_url = sanitize_optional_string(database_url);
    if database_url.is_some() {
        return database_url;
    }
    sanitize_optional_string(configured)
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
    validate_app_config(&config.app)?;
    validate_secret_store_config(&config.secret_store)?;
    validate_storage_config(&config.file_library, &config.chunking)?;
    validate_sources_config(&config.connections, &config.sources)?;
    Ok(())
}

fn sanitize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::resolve_app_db_url;

    fn some(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn database_url_wins_over_configured() {
        assert_eq!(
            resolve_app_db_url(
                some("postgres://default/db"),
                some("postgres://configured/db")
            ),
            some("postgres://default/db")
        );
    }

    #[test]
    fn configured_kept_when_no_database_url() {
        assert_eq!(
            resolve_app_db_url(None, some("postgres://configured/db")),
            some("postgres://configured/db")
        );
    }

    #[test]
    fn blank_database_url_falls_through_to_configured() {
        assert_eq!(
            resolve_app_db_url(some("   "), some("postgres://configured/db")),
            some("postgres://configured/db")
        );
    }

    #[test]
    fn blank_configured_treated_as_absent() {
        assert_eq!(resolve_app_db_url(None, some("  ")), None);
    }

    #[test]
    fn none_is_none() {
        assert_eq!(resolve_app_db_url(None, None), None);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(
            resolve_app_db_url(some("  postgres://default/db  "), None),
            some("postgres://default/db")
        );
    }
}
