use context69_contracts::SearchMode;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct AccessScope {
    pub user_id: Option<i64>,
    pub include_public: bool,
    pub private_group_ids: Vec<i64>,
    pub group_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchSettings {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    pub api_key: Option<String>,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            mode: SearchMode::Hybrid,
            rerank_enabled: true,
            rerank_base_url: "https://openrouter.ai/api/v1".to_string(),
            rerank_model: "cohere/rerank-4-fast".to_string(),
            candidate_limit: 40,
            timeout_secs: 10,
            api_key: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoredRerankItemScore {
    pub rerank_model: String,
    pub query_hash: String,
    pub query_text_trimmed: String,
    pub chunk_id: Uuid,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct SearchPointHit {
    pub chunk_id: Uuid,
    pub score: f32,
}
