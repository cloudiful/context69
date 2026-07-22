use anyhow::{Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::{Client, Response, StatusCode, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::EmbeddingConfig;

const MAX_EMBEDDING_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_EMBEDDING_ERROR_RESPONSE_BYTES: usize = 1024 * 1024;

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

        let response = builder.send().await.map_err(|error| {
            format_embedding_transport_error("send request", &endpoint, &self.config.model, error)
        })?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let max_body_bytes = if status.is_success() {
            MAX_EMBEDDING_RESPONSE_BYTES
        } else {
            MAX_EMBEDDING_ERROR_RESPONSE_BYTES
        };
        let body =
            read_response_body(response, max_body_bytes, &endpoint, &self.config.model).await?;

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

async fn read_response_body(
    response: Response,
    max_bytes: usize,
    endpoint: &str,
    model: &str,
) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(oversized_response_error(max_bytes, endpoint, model, &[]));
    }

    read_response_body_stream(response.bytes_stream(), max_bytes, endpoint, model).await
}

async fn read_response_body_stream<S>(
    mut stream: S,
    max_bytes: usize,
    endpoint: &str,
    model: &str,
) -> Result<String>
where
    S: futures::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            format_embedding_transport_error("read response body", endpoint, model, error)
        })?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(oversized_response_error(max_bytes, endpoint, model, &body));
        }
        body.extend_from_slice(&chunk);
    }

    String::from_utf8(body)
        .map_err(|error| anyhow!("embedding response body is not valid UTF-8: {error}"))
}

fn oversized_response_error(
    max_bytes: usize,
    endpoint: &str,
    model: &str,
    body: &[u8],
) -> anyhow::Error {
    let preview = String::from_utf8_lossy(&body[..body.len().min(320)]);
    anyhow!(
        "embedding response body exceeds {max_bytes} bytes: endpoint={endpoint} model={model} body_preview={preview:?}"
    )
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

fn format_embedding_transport_error(
    operation: &str,
    endpoint: &str,
    model: &str,
    error: reqwest::Error,
) -> anyhow::Error {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_decode() {
        "decode"
    } else {
        "transport"
    };

    anyhow!(
        "embedding upstream transport error: operation={operation} kind={kind} endpoint={endpoint} model={model}: {error}"
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
#[path = "embedding_tests.rs"]
mod tests;
