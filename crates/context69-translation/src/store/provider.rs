use sha2::{Digest, Sha256};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct StoredTranslationProvider {
    pub provider_key: String,
    pub enabled: bool,
    pub priority: i32,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub llm_api_kind: Option<String>,
    pub deepl_plan: Option<String>,
    pub monthly_character_limit: Option<i64>,
}

impl StoredTranslationProvider {
    pub fn config_hash(&self) -> String {
        let mut digest = Sha256::new();
        for value in [
            Some(self.provider_key.as_str()),
            self.endpoint.as_deref(),
            self.model.as_deref(),
            self.llm_api_kind.as_deref(),
            self.deepl_plan.as_deref(),
        ] {
            digest.update(value.unwrap_or_default().as_bytes());
            digest.update(b"\0");
        }
        hex_digest(digest.finalize().as_slice())
    }
}

fn hex_digest(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
