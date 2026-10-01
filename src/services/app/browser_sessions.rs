use anyhow::Result;
use sha2::{Digest, Sha512};

use crate::{
    config::Config,
    services::secret_store::{SecretPurpose, SecretStore},
};

#[derive(Clone)]
pub struct BrowserSessionConfig {
    pub valkey_url: String,
    pub signing_key: [u8; 64],
}

const SIGNING_KEY_NAME: &str = "browser_session_signing_key_v2";
const SIGNING_KEY_LEN: usize = 64;

pub async fn resolve(store: &SecretStore, config: &Config) -> Result<BrowserSessionConfig> {
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
        let mut candidate = [0_u8; SIGNING_KEY_LEN];
        getrandom::fill(&mut candidate)
            .map_err(|error| anyhow::anyhow!("failed to generate browser session key: {error}"))?;
        let stored = store
            .get_or_create(
                SecretPurpose::BrowserSessionSigningKey,
                SIGNING_KEY_NAME,
                &candidate,
            )
            .await?;
        // The store owns the bytes; this layer only has to know they are a
        // signing key. The length is reported, never the value.
        let stored = stored.expose();
        let signing_key: [u8; SIGNING_KEY_LEN] = stored.try_into().map_err(|_| {
            anyhow::anyhow!(
                "internal browser session signing key has invalid length {}; expected {SIGNING_KEY_LEN}",
                stored.len()
            )
        })?;
        signing_key
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
    use super::{SIGNING_KEY_NAME, resolve_valkey_url};
    use crate::{config::Config, services::secret_store::SecretPurpose};

    #[test]
    fn valkey_prefers_override_then_runtime_then_default() {
        let mut config = Config::default();
        assert_eq!(resolve_valkey_url(&config), "redis://127.0.0.1:6379");

        config.scheduler.valkey_url = Some("redis://runtime:6379/0".to_string());
        assert_eq!(resolve_valkey_url(&config), "redis://runtime:6379/0");

        config.auth.session_valkey_url = Some(" redis://override:6379/1 ".to_string());
        assert_eq!(resolve_valkey_url(&config), "redis://override:6379/1");
    }

    #[test]
    fn the_signing_key_row_keeps_its_historical_name() {
        // The stored key is the identity existing deployments already resolved
        // against; renaming it would orphan every installed signing key.
        assert_eq!(SIGNING_KEY_NAME, "browser_session_signing_key_v2");
        assert_eq!(
            SecretPurpose::BrowserSessionSigningKey.as_str(),
            "browser_session.signing_key"
        );
    }
}
