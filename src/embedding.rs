use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use reqwest::{Client, StatusCode, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::EmbeddingConfig;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed_texts(&[query.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("embedding provider returned no vectors"))
    }
}

#[derive(Clone)]
pub struct OpenAiCompatibleEmbeddingProvider {
    client: Client,
    config: EmbeddingConfig,
}

impl OpenAiCompatibleEmbeddingProvider {
    pub fn new(config: EmbeddingConfig) -> Result<Self> {
        let client = Client::builder().timeout(config.timeout).build()?;
        Ok(Self { client, config })
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiCompatibleEmbeddingProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = EmbeddingRequest {
            model: self.config.model.clone(),
            input: texts.to_vec(),
        };

        let endpoint = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));
        let mut builder = self.client.post(&endpoint).json(&request);

        if let Some(api_key) = &self.config.api_key {
            builder = builder.bearer_auth(api_key);
        }

        let response = builder
            .send()
            .await
            .with_context(|| format!("failed to send embedding request to {endpoint}"))?;
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
            .with_context(|| format!("failed to read embedding response body from {endpoint}"))?;

        if !status.is_success() {
            return Err(format_embedding_http_error(
                status,
                &endpoint,
                &self.config.model,
                &content_type,
                &body,
            ));
        }

        let payload =
            parse_embedding_response(&body, &endpoint, &self.config.model, &content_type)?;
        let vectors = payload
            .data
            .into_iter()
            .map(|item| item.embedding)
            .collect::<Vec<_>>();
        if vectors.len() != texts.len() {
            return Err(anyhow!(
                "embedding provider returned {} vectors for {} inputs",
                vectors.len(),
                texts.len()
            ));
        }
        Ok(vectors)
    }
}

#[derive(Debug, Clone, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

fn parse_embedding_response(
    body: &str,
    endpoint: &str,
    model: &str,
    content_type: &str,
) -> Result<EmbeddingResponse> {
    serde_json::from_str(body).map_err(|error| {
        let preview = truncate_for_error(body, 320);
        let embedded_error = extract_error_message(body)
            .map(|message| format!(" provider_error={message}"))
            .unwrap_or_default();
        anyhow!(
            "failed to parse embedding response: endpoint={endpoint} model={model} content_type={content_type} body_preview={preview:?}{embedded_error}: {error}"
        )
    })
}

fn format_embedding_http_error(
    status: StatusCode,
    endpoint: &str,
    model: &str,
    content_type: &str,
    body: &str,
) -> anyhow::Error {
    let preview = truncate_for_error(body, 320);
    let embedded_error = extract_error_message(body)
        .map(|message| format!(" provider_error={message}"))
        .unwrap_or_default();
    anyhow!(
        "embedding request failed: status={status} endpoint={endpoint} model={model} content_type={content_type} body_preview={preview:?}{embedded_error}"
    )
}

fn extract_error_message(body: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(body).ok()?;

    match value.get("error") {
        Some(Value::String(message)) => Some(message.clone()),
        Some(Value::Object(map)) => map
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| Some(Value::Object(map.clone()).to_string())),
        Some(other) => Some(other.to_string()),
        None => None,
    }
}

fn truncate_for_error(input: &str, max_chars: usize) -> String {
    let mut truncated = input.chars().take(max_chars).collect::<String>();
    if input.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::{extract_error_message, format_embedding_http_error, parse_embedding_response};

    #[test]
    fn parse_error_includes_response_context() {
        let error = parse_embedding_response(
            "<html>upstream failure</html>",
            "http://127.0.0.1:11434/v1/embeddings",
            "nomic-embed-text",
            "text/html",
        )
        .expect_err("html should not parse as embedding response");

        let message = error.to_string();
        assert!(message.contains("failed to parse embedding response"));
        assert!(message.contains("endpoint=http://127.0.0.1:11434/v1/embeddings"));
        assert!(message.contains("content_type=text/html"));
        assert!(message.contains("body_preview"));
    }

    #[test]
    fn http_error_extracts_provider_error_message() {
        let error = format_embedding_http_error(
            StatusCode::BAD_REQUEST,
            "http://127.0.0.1:11434/v1/embeddings",
            "nomic-embed-text",
            "application/json",
            r#"{"error":{"message":"model not found"}}"#,
        );

        let message = error.to_string();
        assert!(message.contains("status=400 Bad Request"));
        assert!(message.contains("provider_error=model not found"));
    }

    #[test]
    fn extracts_top_level_error_string() {
        assert_eq!(
            extract_error_message(r#"{"error":"backend unavailable"}"#).as_deref(),
            Some("backend unavailable")
        );
    }
}
