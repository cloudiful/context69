use anyhow::Result;
use tracing::warn;

use crate::{
    config::{Config, ConnectionConfig},
    db::Database,
    services::{app::browser_sessions, secret_store::SecretStore, settings::SettingsService},
};

pub struct ConfigHydration {
    pub settings: SettingsService,
    pub secrets: SecretStore,
    pub browser_sessions: browser_sessions::BrowserSessionConfig,
    pub runtime_configured: bool,
}

pub async fn hydrate(db: &Database, config: &mut Config) -> Result<ConfigHydration> {
    super::runtime_settings::import_legacy_runtime_if_needed(db, config).await?;
    let settings = SettingsService::new(db.clone());
    let runtime = super::runtime_settings::load_runtime_settings(db).await?;
    if let Some(runtime) = &runtime {
        super::runtime_settings::apply_runtime_settings(config, runtime);
    }
    // The secret store is built before anything reads a persisted secret, so
    // browser-session resolution already goes through the typed accessor.
    let secrets = SecretStore::new(db.clone(), &config.secret_store)?;
    let browser_sessions = browser_sessions::resolve(&secrets, config).await?;
    config.connections = db
        .list_source_connections()
        .await?
        .into_iter()
        .map(|connection| ConnectionConfig {
            name: connection.name,
            database_url: connection.database_url,
        })
        .collect();
    config.docling = match settings.resolve_docling_config().await {
        Ok(docling) => docling,
        Err(error) => {
            warn!(error = %error, "docling settings are invalid; continuing without docling runtime");
            None
        }
    };

    Ok(ConfigHydration {
        settings,
        secrets,
        browser_sessions,
        runtime_configured: runtime.is_some(),
    })
}
