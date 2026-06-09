use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::{Client, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::db::StoredSearchSettings;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankDocument {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankHit {
    pub index: usize,
    pub score: f32,
}

#[derive(Clone)]
pub struct OpenRouterRerankClient {
    client: Client,
}

impl OpenRouterRerankClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder().build()?,
        })
    }

    pub async fn rerank(
        &self,
        query: &str,
        documents: &[RerankDocument],
        top_n: usize,
        settings: &StoredSearchSettings,
    ) -> Result<Vec<RerankHit>> {
        let api_key = settings
            .api_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .context("rerank api key is not configured")?;
        if documents.is_empty() {
            return Ok(Vec::new());
        }

        let endpoint = format!("{}/rerank", settings.rerank_base_url.trim_end_matches('/'));
        let request = RerankRequest {
            model: settings.rerank_model.clone(),
            query: query.to_string(),
            documents: documents
                .iter()
                .map(|document| document.text.clone())
                .collect(),
            top_n,
        };

        let response = self
            .client
            .post(&endpoint)
            .timeout(Duration::from_secs(settings.timeout_secs))
            .bearer_auth(api_key)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("failed to send rerank request to {endpoint}"))?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let body = response
            .text()
            .await
            .with_context(|| format!("failed to read rerank response body from {endpoint}"))?;

        if !status.is_success() {
            return Err(anyhow!(
                "rerank request failed: status={status} endpoint={endpoint} content_type={content_type} body_preview={:?}",
                body.chars().take(500).collect::<String>()
            ));
        }

        let payload: RerankResponse = serde_json::from_str(&body).with_context(|| {
            format!(
                "failed to parse rerank response: endpoint={endpoint} content_type={content_type} body_preview={:?}",
                body.chars().take(500).collect::<String>()
            )
        })?;

        payload
            .results
            .into_iter()
            .map(|result| {
                let index = result
                    .index
                    .or_else(|| result.document.and_then(|document| document.index))
                    .context("rerank result missing index")?;
                Ok(RerankHit {
                    index,
                    score: result.relevance_score,
                })
            })
            .collect()
    }
}

#[derive(Debug, Serialize)]
struct RerankRequest {
    model: String,
    query: String,
    documents: Vec<String>,
    top_n: usize,
}

#[derive(Debug, Deserialize)]
struct RerankResponse {
    results: Vec<RerankResponseResult>,
}

#[derive(Debug, Deserialize)]
struct RerankResponseResult {
    #[serde(default)]
    index: Option<usize>,
    relevance_score: f32,
    #[serde(default)]
    document: Option<RerankResponseDocument>,
}

#[derive(Debug, Deserialize)]
struct RerankResponseDocument {
    #[serde(default)]
    index: Option<usize>,
}
