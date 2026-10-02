use anyhow::Result;
use tracing::warn;

use crate::{
    config::{Config, ConnectionConfig},
    db::Database,
    services::{app::browser_sessions, secret_store, settings::SettingsService},
};

pub struct ConfigHydration {
    pub settings: SettingsService,
    pub secrets: secret_store::SecretStore,
    pub browser_sessions: browser_sessions::BrowserSessionConfig,
    pub runtime_configured: bool,
}

pub async fn hydrate(db: &Database, config: &mut Config) -> Result<ConfigHydration> {
    // The store is built first, so every read below can resolve a secret through
    // it: the legacy bootstrap import, the persisted settings load, and
    // browser-session resolution all see the same accessor.
    let secrets = secret_store::build(db, config)?;
    if !secrets.is_encrypted() {
        // Reported once per process, here, rather than by every handle built onto
        // the same configuration.
        warn!(
            "app.master_secret is not configured; sealed secrets cannot be opened and new \
             secrets are stored in the legacy plaintext representation"
        );
    }
    super::runtime_settings::import_legacy_runtime_if_needed(db, config, &secrets).await?;
    let settings = SettingsService::with_secrets(db.clone(), secrets.clone());
    let runtime = super::runtime_settings::load_runtime_settings(db, &secrets).await?;
    if let Some(runtime) = &runtime {
        super::runtime_settings::apply_runtime_settings(config, runtime);
    }
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
