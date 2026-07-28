use sha2::{Digest, Sha256};

use crate::config::Config;

pub fn fingerprint(config: &Config) -> String {
    digest_parts([
        config.qdrant.url.trim_end_matches('/').to_string(),
        config.qdrant.collection_name.clone(),
        config.embedding.base_url.trim_end_matches('/').to_string(),
        config.embedding.model.clone(),
        config.embedding.dimensions.to_string(),
    ])
}

pub fn configuration_fingerprint(config: &Config) -> String {
    digest_parts([
        "embedding_vector".to_string(),
        config.qdrant.url.trim_end_matches('/').to_string(),
        config.qdrant.collection_name.clone(),
        config.embedding.base_url.trim_end_matches('/').to_string(),
        config.embedding.api_key.clone().unwrap_or_default(),
        config.embedding.model.clone(),
        config.embedding.dimensions.to_string(),
    ])
}

fn digest_parts(parts: impl IntoIterator<Item = String>) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::fingerprint;
    use crate::config::Config;

    #[test]
    fn tracks_embedding_identity() {
        let mut config = Config::default();
        let original = fingerprint(&config);

        config.embedding.model = "different-model".to_string();
        assert_ne!(fingerprint(&config), original);

        config.embedding.model = Config::default().embedding.model;
        config.embedding.dimensions += 1;
        assert_ne!(fingerprint(&config), original);

        config.embedding.dimensions = Config::default().embedding.dimensions;
        config.embedding.base_url = "https://different.example/v1".to_string();
        assert_ne!(fingerprint(&config), original);

        config.embedding.base_url = Config::default().embedding.base_url;
        config.qdrant.collection_name = "different-collection".to_string();
        assert_ne!(fingerprint(&config), original);
    }

    #[test]
    fn tracks_configuration_credentials_without_exposing_them() {
        let mut config = Config::default();
        let original = configuration_fingerprint(&config);
        config.embedding.api_key = Some("different-secret".to_string());
        let changed = configuration_fingerprint(&config);

        assert_ne!(changed, original);
        assert!(!changed.contains("different-secret"));
    }
}
