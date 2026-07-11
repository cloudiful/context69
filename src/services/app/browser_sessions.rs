use anyhow::Result;
use sha2::{Digest, Sha512};

use crate::{config::Config, db::Database};

#[derive(Clone)]
pub struct BrowserSessionConfig {
    pub valkey_url: String,
    pub signing_key: [u8; 64],
}

const SIGNING_KEY_NAME: &str = "browser_session_signing_key_v2";

pub async fn resolve(db: &Database, config: &Config) -> Result<BrowserSessionConfig> {
    let valkey_url = resolve_valkey_url(config);
    let signing_key = if let Some(secret) = config
        .auth
        .session_secret_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Sha512::digest(secret.as_bytes()).into()
    } else {
        let mut candidate = [0_u8; 64];
        getrandom::fill(&mut candidate)
            .map_err(|error| anyhow::anyhow!("failed to generate browser session key: {error}"))?;
        let stored = db
            .get_or_create_internal_secret(SIGNING_KEY_NAME, &candidate)
            .await?;
        stored.try_into().map_err(|stored: Vec<u8>| {
            anyhow::anyhow!(
                "internal browser session signing key has invalid length {}; expected 64",
                stored.len()
            )
        })?
    };

    Ok(BrowserSessionConfig {
        valkey_url,
        signing_key,
    })
}

fn resolve_valkey_url(config: &Config) -> String {
    config
        .auth
        .session_valkey_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| config.scheduler.valkey_url.clone())
        .unwrap_or_else(|| crate::config::DEFAULT_SESSION_VALKEY_URL.to_string())
}

#[cfg(test)]
mod tests {
    use super::resolve_valkey_url;
    use crate::config::Config;

    #[test]
    fn valkey_prefers_override_then_runtime_then_default() {
        let mut config = Config::default();
        assert_eq!(resolve_valkey_url(&config), "redis://127.0.0.1:6379");

        config.scheduler.valkey_url = Some("redis://runtime:6379/0".to_string());
        assert_eq!(resolve_valkey_url(&config), "redis://runtime:6379/0");

        config.auth.session_valkey_url = Some(" redis://override:6379/1 ".to_string());
        assert_eq!(resolve_valkey_url(&config), "redis://override:6379/1");
    }
}
