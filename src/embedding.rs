use std::{sync::Arc, time::Instant};

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::{Client, Response, header::CONTENT_TYPE};
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::{config::EmbeddingConfig, domain_errors::DomainError, retry};

mod errors;
#[path = "embedding/retry.rs"]
mod retry_support;

#[path = "embedding_batch.rs"]
pub mod embedding_batch;
#[path = "embedding_runtime.rs"]
pub mod embedding_runtime;

pub use embedding_batch::{EmbeddingBatch, EmbeddingBatchLimits};
pub use embedding_runtime::{
    EmbeddingIdentity, EmbeddingRuntime, build_probe_provider, build_probe_provider_with_config,
    build_provider,
};

use errors::{
    PREVIEW_CHARS, bound_provider_error, extract_error_message, format_embedding_attempt_timeout,
    format_embedding_http_error, format_embedding_retry_budget_error,
    format_embedding_transport_error, oversized_response_error, redact_then_truncate,
    sanitize_endpoint,
};
use retry_support::{finalize_error, retry_deadline};

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
            .ok_or_else(|| DomainError::upstream_error("embedding provider returned no vectors"))
            .map_err(anyhow::Error::from)
    }

    /// The model this provider embeds a query with, when it uses a different
    /// one than it embeds documents with.
    ///
    /// A provider without a distinct query model returns `None`: its queries
    /// are embedded with the document model. The value scopes the cached query
    /// vectors, so a changed query alias invalidates that cache and nothing
    /// else.
    fn effective_query_model(&self) -> Option<&str> {
        None
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

    /// The model queries are embedded with: the configured alias, or the
    /// document model when no alias is configured.
    fn query_model(&self) -> &str {
        self.config
            .query_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(self.config.model.as_str())
    }

    /// The limits one request is planned against: the fixed transport packing.
    fn effective_limits(&self) -> EmbeddingBatchLimits {
        EmbeddingBatchLimits::fixed()
    }

    async fn embed_texts_with_model(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let started = Instant::now();
        // One deadline covers the whole operation, so a long input array cannot
        // multiply the provider's timeout budget.
        let deadline = retry_deadline(started, self.config.timeout);

        let limits = self.effective_limits();
        let batches = embedding_batch::plan_batches(texts, model, &limits)?;

        let endpoint = Arc::new(format!(
            "{}/embeddings",
            self.config.base_url.trim_end_matches('/')
        ));
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in &batches {
            let batch_vectors = self
                .embed_batch(batch.texts(texts), model, &endpoint, deadline, started)
                .await?;
            vectors.extend(batch_vectors);
        }
        Ok(vectors)
    }

    /// Sends one planned request, retrying it under the shared deadline.
    async fn embed_batch(
        &self,
        texts: &[String],
        model: &str,
        endpoint: &Arc<String>,
        deadline: Instant,
        started: Instant,
    ) -> Result<Vec<Vec<f32>>> {
        let request = Arc::new(embedding_batch::EmbeddingRequest {
            model: model.to_string(),
            input: texts.to_vec(),
        });
        let mut attempts = 0;
        let result = retry::retry_until(
            deadline,
            "embedding request",
            format_embedding_retry_budget_error,
            retry::is_retryable,
            || {
                attempts += 1;
                let attempt = attempts;
                let request = Arc::clone(&request);
                let endpoint = Arc::clone(endpoint);
                let attempt_deadline = tokio::time::Instant::from_std(deadline);
                async move {
                    match tokio::time::timeout_at(
                        attempt_deadline,
                        self.embed_once(&request, endpoint.as_str(), texts.len(), model),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(format_embedding_attempt_timeout(
                            endpoint.as_str(),
                            model,
                            attempt,
                        )),
                    }
                }
            },
        )
        .await;

        match result {
            Ok(vectors) => Ok(vectors),
            Err(error) => {
                let error = finalize_error(error, attempts, started, self.config.timeout);
                error!(
                    batch_size = texts.len(),
                    attempts,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    error = %error,
                    "embedding batch failed"
                );
                Err(error)
            }
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiCompatibleEmbeddingProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed_texts_with_model(texts, &self.config.model).await
    }

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let model = self.query_model().to_string();
        let embeddings = self
            .embed_texts_with_model(&[query.to_string()], &model)
            .await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| DomainError::upstream_error("embedding provider returned no vectors"))
            .map_err(anyhow::Error::from)
    }

    fn effective_query_model(&self) -> Option<&str> {
        let query_model = self.query_model();
        (query_model != self.config.model).then_some(query_model)
    }
}

impl OpenAiCompatibleEmbeddingProvider {
    async fn embed_once(
        &self,
        request: &embedding_batch::EmbeddingRequest,
        endpoint: &str,
        input_count: usize,
        model: &str,
    ) -> Result<Vec<Vec<f32>>> {
        let started = Instant::now();
        let request_bytes = serde_json::to_vec(request)
            .map(|body| body.len())
            .unwrap_or_default();
        let request_chars: usize = request.input.iter().map(|text| text.chars().count()).sum();

        let mut builder = self.client.post(endpoint).json(request);

        if let Some(api_key) = &self.config.api_key {
            builder = builder.bearer_auth(api_key);
        }

        let response = builder.send().await.map_err(|error| {
            format_embedding_transport_error(
                "send request",
                endpoint,
                model,
                error,
                self.config.api_key.as_deref(),
            )
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
        let body = read_response_body(
            response,
            max_body_bytes,
            endpoint,
            model,
            self.config.api_key.as_deref(),
        )
        .await?;

        if !status.is_success() {
            return Err(format_embedding_http_error(
                status,
                endpoint,
                model,
                &content_type,
                &body,
                self.config.api_key.as_deref(),
            ));
        }

        let payload = parse_embedding_response(
            &body,
            endpoint,
            model,
            &content_type,
            self.config.api_key.as_deref(),
        )?;
        let vectors = ordered_vectors(payload.data, input_count)?;
        // One line per request that actually left this deployment. It carries the
        // shape of the request and nothing about its content: no text, no
        // credential, no header, and no URL.
        info!(
            items = input_count,
            chars = request_chars,
            request_bytes,
            elapsed_ms = started.elapsed().as_millis() as u64,
            vectors_returned = vectors.len(),
            prompt_tokens = payload.usage.as_ref().and_then(|usage| usage.prompt_tokens),
            total_tokens = payload.usage.as_ref().and_then(|usage| usage.total_tokens),
            model,
            "embedding request completed"
        );
        Ok(vectors)
    }
}

/// The vectors of one response, in the order of the inputs they belong to.
///
/// A provider that indexes its results is authoritative about the order, so the
/// vectors are placed by their index. A provider that does not index them is
/// accepted in the order it sent them, as long as the count matches. A
/// response that is partly indexed, or that repeats or overruns an index, is
/// refused: guessing the order would silently attach a vector to the wrong
/// input.
fn ordered_vectors(data: Vec<EmbeddingData>, input_count: usize) -> Result<Vec<Vec<f32>>> {
    let indexed = data.iter().filter(|item| item.index.is_some()).count();
    if indexed == 0 {
        if data.len() != input_count {
            return Err(DomainError::internal(format!(
                "embedding provider returned {} vectors for {} inputs",
                data.len(),
                input_count
            ))
            .into());
        }
        warn!(
            vectors = data.len(),
            "embedding provider returned no per-item index; the response order is used as sent"
        );
        return Ok(data.into_iter().map(|item| item.embedding).collect());
    }
    if indexed != data.len() {
        return Err(DomainError::internal(
            "embedding provider returned a partly indexed response; the vector order cannot be \
             determined",
        )
        .into());
    }
    if data.len() != input_count {
        return Err(DomainError::internal(format!(
            "embedding provider returned {} vectors for {} inputs",
            data.len(),
            input_count
        ))
        .into());
    }

    let mut ordered: Vec<Option<Vec<f32>>> = vec![None; input_count];
    for item in data {
        let index = item
            .index
            .expect("every item of a fully indexed response carries an index");
        if index < 0 || index as usize >= input_count {
            return Err(DomainError::internal(format!(
                "embedding provider returned the index {index} for {input_count} inputs"
            ))
            .into());
        }
        let slot = index as usize;
        if ordered[slot].is_some() {
            return Err(DomainError::internal(format!(
                "embedding provider returned the index {index} more than once"
            ))
            .into());
        }
        ordered[slot] = Some(item.embedding);
    }
    Ok(ordered
        .into_iter()
        .map(|vector| vector.expect("every input has one vector"))
        .collect())
}

async fn read_response_body(
    response: Response,
    max_bytes: usize,
    endpoint: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(oversized_response_error(
            max_bytes,
            endpoint,
            model,
            &[],
            api_key,
        ));
    }

    read_response_body_stream(response.bytes_stream(), max_bytes, endpoint, model, api_key).await
}

async fn read_response_body_stream<S>(
    mut stream: S,
    max_bytes: usize,
    endpoint: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<String>
where
    S: futures::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            format_embedding_transport_error("read response body", endpoint, model, error, api_key)
        })?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(oversized_response_error(
                max_bytes, endpoint, model, &body, api_key,
            ));
        }
        body.extend_from_slice(&chunk);
    }

    String::from_utf8(body).map_err(|error| {
        anyhow::Error::from(DomainError::internal(format!(
            "embedding response body is not valid UTF-8: {error}"
        )))
    })
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    #[serde(default)]
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddingData {
    /// The position of this vector in the request. A provider that does not
    /// index its results leaves it absent.
    #[serde(default)]
    index: Option<i64>,
    embedding: Vec<f32>,
}

/// The token usage a provider reported, when it reports one.
#[derive(Debug, Clone, Deserialize)]
struct EmbeddingUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

fn parse_embedding_response(
    body: &str,
    endpoint: &str,
    model: &str,
    content_type: &str,
    api_key: Option<&str>,
) -> Result<EmbeddingResponse> {
    serde_json::from_str(body).map_err(|error| {
        let preview = redact_then_truncate(body, PREVIEW_CHARS, api_key);
        let embedded_error = extract_error_message(body)
            .map(|message| format!(" provider_error={}", bound_provider_error(&message, api_key)))
            .unwrap_or_default();
        anyhow::Error::from(DomainError::internal(format!(
            "failed to parse embedding response: endpoint={} model={model} content_type={content_type} body_preview={preview:?}{embedded_error}: {error}",
            sanitize_endpoint(endpoint)
        )))
    })
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
