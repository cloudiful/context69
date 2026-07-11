use sha2::{Digest, Sha256};

use crate::config::Config;

pub fn fingerprint(config: &Config) -> String {
    let mut digest = Sha256::new();
    digest.update(config.embedding.base_url.trim_end_matches('/').as_bytes());
    digest.update([0]);
    digest.update(config.embedding.model.as_bytes());
    digest.update([0]);
    digest.update(config.embedding.dimensions.to_le_bytes());
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
    }
}
