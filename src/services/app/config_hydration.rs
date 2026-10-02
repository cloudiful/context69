use anyhow::Result;
use tracing::warn;

use crate::{
    config::{Config, ConnectionConfig},
    db::Database,
    services::{
        app::browser_sessions,
        secret_store::{self, SecretPurpose},
        settings::{SettingsService, secrets::resolve_stored},
    },
};

pub struct ConfigHydration {
    pub settings: SettingsService,
    pub secrets: secret_store::SecretStore,
    pub browser_sessions: browser_sessions::BrowserSessionConfig,
    pub runtime_configured: bool,
}

pub async fn hydrate(db: &Database, config: &mut Config) -> Result<ConfigHydration> {
    // The store is built first, so every read below can resolve a secret through
    // it: the first-boot import, the persisted settings load, the source
    // connections, and browser-session resolution all see the same accessor.
    let secrets = secret_store::build(db, config)?;
    if !secrets.is_encrypted() {
        // Reported once per process, here, rather than by every handle built onto
        // the same configuration.
        warn!(
            "app.master_secret is not configured; sealed secrets cannot be opened and new \
             secrets are stored unsealed"
        );
    }
    super::runtime_settings::import_legacy_runtime_if_needed(db, config, &secrets).await?;
    let settings = SettingsService::with_secrets(db.clone(), secrets.clone());
    let runtime = super::runtime_settings::load_runtime_settings(db, &secrets).await?;
    if let Some(runtime) = &runtime {
        super::runtime_settings::apply_runtime_settings(config, runtime);
    }
    let browser_sessions = browser_sessions::resolve(&secrets, config).await?;
    // A connection's database URL lives only in the shared store, so the effective
    // configuration is built from the value this deployment can actually open. A
    // connection whose sealed URL is unavailable is left out and reported, the same
    // way an invalid Docling configuration is, rather than being turned into an
    // absent or placeholder credential.
    let mut connections = Vec::new();
    for connection in db.list_source_connections().await? {
        let Some(secret_key) = connection.database_url_secret_key.as_deref() else {
            continue;
        };
        match resolve_stored(
            &secrets,
            SecretPurpose::SourceConnectionDatabaseUrl,
            secret_key,
        )
        .await
        {
            Ok(Some(database_url)) => connections.push(ConnectionConfig {
                name: connection.name,
                database_url,
            }),
            Ok(None) => {}
            Err(error) => warn!(
                connection = connection.name,
                error = %error,
                "source connection credential is unavailable; leaving it out of the effective \
                 configuration"
            ),
        }
    }
    config.connections = connections;
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
