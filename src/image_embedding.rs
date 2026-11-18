//! The optional OpenAI-compatible image embedding provider.
//!
//! Image inputs are sent as bounded homogeneous batches of
//! `{"bytes": "<base64>"}` objects. Text queries are sent as a homogeneous
//! array of strings. The two input shapes are never mixed in one request: the
//! gateway compatibility of a mixed array is unverified, so the provider only
//! sends a shape it can validate.
//!
//! Every response is checked before it can reach a caller: the vector count
//! must equal the input count, each vector must have the configured dimension,
//! and every component must be finite. An invalid response fails the whole
//! batch instead of storing a partial or malformed point.

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::ImageEmbeddingConfig;

/// The largest number of images one request may carry.
pub const IMAGE_EMBEDDING_MAX_BATCH_ITEMS: usize = 8;

/// The largest total image byte size one request may carry before base64
/// expansion.
pub const IMAGE_EMBEDDING_MAX_BATCH_BYTES: usize = 20 * 1024 * 1024;

/// The largest error body retained from a failed provider response.
const MAX_ERROR_BODY_BYTES: usize = 4096;

/// The fixed image input policy identity.
///
/// Raw image bytes are sent with no text prefix, caption, or special token.
/// Keeping this marker inside the vector identity means a later policy change
/// cannot silently relabel existing image vectors.
pub const IMAGE_INPUT_POLICY_IDENTITY: &str =
    "image-input-policy-v1:raw-bytes:no-text-prefix:no-caption";

/// The embedding channel of one image query or one image batch.
#[async_trait::async_trait]
pub trait ImageEmbeddingProvider: Send + Sync {
    /// Embed a homogeneous batch of image byte payloads.
    async fn embed_images(&self, images: &[Vec<u8>]) -> Result<Vec<Vec<f32>>>;

    /// Embed one text query into the same vector space.
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>>;
}

/// An OpenAI-compatible image embedding provider.
#[derive(Clone)]
pub struct OpenAiCompatibleImageEmbeddingProvider {
    client: Client,
    config: ImageEmbeddingConfig,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    #[serde(default)]
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddingData {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    embedding: Vec<f32>,
}

impl OpenAiCompatibleImageEmbeddingProvider {
    pub fn new(config: ImageEmbeddingConfig) -> Result<Self> {
        let client = Client::builder().timeout(config.timeout).build()?;
        Ok(Self { client, config })
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/embeddings",
            self.config.base_url.trim().trim_end_matches('/')
        )
    }

    async fn request(
        &self,
        model: &str,
        input: serde_json::Value,
        expected_count: usize,
    ) -> Result<Vec<Vec<f32>>> {
        let endpoint = self.endpoint();
        let request = EmbeddingRequest { model, input };
        let mut builder = self.client.post(&endpoint).json(&request);
        if let Some(api_key) = self
            .config
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            builder = builder.bearer_auth(api_key);
        }
        let response = builder
            .send()
            .await
            .with_context(|| format!("image embedding request to {endpoint} failed"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .bytes()
                .await
                .map(|bytes| {
                    let limit = bytes.len().min(MAX_ERROR_BODY_BYTES);
                    String::from_utf8_lossy(&bytes[..limit]).into_owned()
                })
                .unwrap_or_default();
            return Err(anyhow!(
                "image embedding provider rejected {expected_count} input(s) at {endpoint} with \
                 status {status}: {body}"
            ));
        }
        let parsed: EmbeddingResponse = response.json().await.with_context(|| {
            format!("image embedding response from {endpoint} is not valid JSON")
        })?;
        validate_vectors(
            parsed.data,
            expected_count,
            self.config.dimensions,
            &endpoint,
        )
    }
}

fn validate_vectors(
    mut data: Vec<EmbeddingData>,
    expected_count: usize,
    dimensions: usize,
    endpoint: &str,
) -> Result<Vec<Vec<f32>>> {
    if data.len() != expected_count {
        return Err(anyhow!(
            "image embedding provider returned {} vector(s) for {expected_count} input(s) at \
             {endpoint}",
            data.len()
        ));
    }
    let indexed = data.iter().filter(|item| item.index.is_some()).count();
    if indexed != 0 && indexed != data.len() {
        return Err(anyhow!(
            "image embedding provider returned a partial index set at {endpoint}: {indexed} of {} \
             vector(s) carry an index",
            data.len()
        ));
    }
    if indexed == data.len() && !data.is_empty() {
        let mut indexes = Vec::with_capacity(data.len());
        for item in &data {
            let index = item.index.expect("every item carries an index");
            if index >= expected_count {
                return Err(anyhow!(
                    "image embedding provider index {index} is out of range for {expected_count} \
                     input(s) at {endpoint}"
                ));
            }
            indexes.push(index);
        }
        indexes.sort_unstable();
        if indexes != (0..expected_count).collect::<Vec<_>>() {
            return Err(anyhow!(
                "image embedding provider indexes at {endpoint} are not exactly 0..{}; got \
                 {indexes:?}",
                expected_count.saturating_sub(1)
            ));
        }
        data.sort_by_key(|item| item.index.expect("every item carries an index"));
    }
    data.into_iter()
        .enumerate()
        .map(|(position, item)| {
            if item.embedding.len() != dimensions {
                return Err(anyhow!(
                    "image embedding dimension mismatch at position {position}: expected \
                     {dimensions}, got {}",
                    item.embedding.len()
                ));
            }
            if !item.embedding.iter().all(|value| value.is_finite()) {
                return Err(anyhow!(
                    "image embedding contains a non-finite value at position {position}"
                ));
            }
            Ok(item.embedding)
        })
        .collect()
}

#[async_trait::async_trait]
impl ImageEmbeddingProvider for OpenAiCompatibleImageEmbeddingProvider {
    async fn embed_images(&self, images: &[Vec<u8>]) -> Result<Vec<Vec<f32>>> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        if images.len() > IMAGE_EMBEDDING_MAX_BATCH_ITEMS {
            return Err(anyhow!(
                "an image embedding request carries at most {IMAGE_EMBEDDING_MAX_BATCH_ITEMS} \
                 images"
            ));
        }
        let total_bytes: usize = images.iter().map(Vec::len).sum();
        if total_bytes > IMAGE_EMBEDDING_MAX_BATCH_BYTES {
            return Err(anyhow!(
                "an image embedding request carries at most {IMAGE_EMBEDDING_MAX_BATCH_BYTES} \
                 image bytes"
            ));
        }
        let input = serde_json::Value::Array(
            images
                .iter()
                .map(|bytes| json!({ "bytes": STANDARD.encode(bytes) }))
                .collect(),
        );
        self.request(&self.config.model, input, images.len()).await
    }

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let model = self
            .config
            .query_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(self.config.model.as_str());
        let input = serde_json::Value::Array(vec![serde_json::Value::String(query.to_string())]);
        self.request(model, input, 1)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("image embedding provider returned no query vector"))
    }
}

/// Builds the provider from one image configuration.
pub fn build_image_provider(
    config: &ImageEmbeddingConfig,
) -> Result<std::sync::Arc<dyn ImageEmbeddingProvider>> {
    Ok(std::sync::Arc::new(
        OpenAiCompatibleImageEmbeddingProvider::new(config.clone())?,
    ))
}

/// The deterministic identity of one image vector space.
///
/// It includes the endpoint, image model, effective query model, dimensions,
/// and the fixed input policy. The API key, timeout, and physical collection
/// name are deliberately excluded: they do not change the vectors a provider
/// produces.
pub fn image_embedding_fingerprint(config: &ImageEmbeddingConfig) -> String {
    use sha2::{Digest, Sha256};

    let query_model = config
        .query_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| config.model.trim());
    let parts = [
        config.base_url.trim().trim_end_matches('/').to_string(),
        config.model.trim().to_string(),
        query_model.to_string(),
        config.dimensions.to_string(),
        IMAGE_INPUT_POLICY_IDENTITY.to_string(),
    ];
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
    use std::time::Duration;

    use super::{
        EmbeddingData, IMAGE_EMBEDDING_MAX_BATCH_BYTES, IMAGE_EMBEDDING_MAX_BATCH_ITEMS,
        image_embedding_fingerprint, validate_vectors,
    };

    fn config() -> crate::config::ImageEmbeddingConfig {
        crate::config::ImageEmbeddingConfig {
            base_url: "http://image-embedding.invalid/v1/".to_string(),
            api_key: Some("first-key".to_string()),
            model: "image-model".to_string(),
            query_model: Some("image-query".to_string()),
            dimensions: 768,
            timeout: Duration::from_secs(30),
            collection_name: "context69_images".to_string(),
        }
    }

    #[test]
    fn fingerprint_ignores_credentials_timeouts_and_collection() {
        let base = config();
        let mut changed = config();
        changed.api_key = Some("another-key".to_string());
        changed.timeout = Duration::from_secs(90);
        changed.collection_name = "another_collection".to_string();
        assert_eq!(
            image_embedding_fingerprint(&base),
            image_embedding_fingerprint(&changed)
        );
    }

    #[test]
    fn fingerprint_changes_with_the_vector_space_identity() {
        let base = config();
        let fingerprint = image_embedding_fingerprint(&base);
        let identity_changes: [fn(&mut crate::config::ImageEmbeddingConfig); 4] = [
            |config| config.base_url = "http://other.invalid/v1".to_string(),
            |config| config.model = "other-image-model".to_string(),
            |config| config.query_model = Some("other-query".to_string()),
            |config| config.dimensions = 1024,
        ];
        for change in identity_changes {
            let mut changed = config();
            change(&mut changed);
            assert_ne!(fingerprint, image_embedding_fingerprint(&changed));
        }
    }

    #[test]
    fn an_absent_query_model_uses_the_image_model_identity() {
        let mut absent = config();
        absent.query_model = None;
        let mut explicit = config();
        explicit.query_model = Some("image-model".to_string());
        assert_eq!(
            image_embedding_fingerprint(&absent),
            image_embedding_fingerprint(&explicit)
        );
    }

    #[test]
    fn vector_validation_rejects_count_dimension_and_non_finite_values() {
        let data = || {
            vec![
                EmbeddingData {
                    index: Some(0),
                    embedding: vec![0.1, 0.2],
                },
                EmbeddingData {
                    index: Some(1),
                    embedding: vec![0.3, 0.4],
                },
            ]
        };
        assert_eq!(
            validate_vectors(data(), 2, 2, "endpoint").expect("valid"),
            vec![vec![0.1, 0.2], vec![0.3, 0.4]]
        );
        assert!(validate_vectors(data(), 1, 2, "endpoint").is_err());
        assert!(validate_vectors(data(), 2, 3, "endpoint").is_err());

        let mut non_finite = data();
        non_finite[0].embedding[0] = f32::NAN;
        assert!(validate_vectors(non_finite, 2, 2, "endpoint").is_err());
    }

    #[test]
    fn vector_indexes_are_reordered_only_when_they_are_complete_and_unique() {
        let indexed = |indexes: [Option<usize>; 3]| {
            indexes
                .into_iter()
                .enumerate()
                .map(|(position, index)| EmbeddingData {
                    index,
                    embedding: vec![position as f32, 1.0],
                })
                .collect::<Vec<_>>()
        };
        let ordered = validate_vectors(indexed([Some(2), Some(0), Some(1)]), 3, 2, "endpoint")
            .expect("a complete unique index set reorders the vectors");
        assert_eq!(
            ordered,
            vec![vec![1.0, 1.0], vec![2.0, 1.0], vec![0.0, 1.0]]
        );

        let absent = indexed([None, None, None]);
        assert_eq!(
            validate_vectors(absent, 3, 2, "endpoint").expect("absent indexes keep order"),
            vec![vec![0.0, 1.0], vec![1.0, 1.0], vec![2.0, 1.0]]
        );

        for invalid in [
            indexed([Some(0), None, Some(2)]),
            indexed([Some(0), Some(0), Some(2)]),
            indexed([Some(0), Some(1), Some(3)]),
            indexed([Some(0), Some(1), Some(2)])
                .into_iter()
                .take(2)
                .collect(),
        ] {
            assert!(validate_vectors(invalid, 3, 2, "endpoint").is_err());
        }
    }

    #[test]
    fn vector_index_validation_rejects_negative_and_gapped_sets() {
        let mut negative = vec![
            EmbeddingData {
                index: Some(0),
                embedding: vec![0.1, 0.2],
            },
            EmbeddingData {
                index: Some(0),
                embedding: vec![0.3, 0.4],
            },
        ];
        assert!(validate_vectors(negative.clone(), 2, 2, "endpoint").is_err());
        negative[0].index = Some(5);
        assert!(validate_vectors(negative, 2, 2, "endpoint").is_err());
    }

    #[test]
    fn declared_batch_limits_are_explicit() {
        assert_eq!(IMAGE_EMBEDDING_MAX_BATCH_ITEMS, 8);
        assert_eq!(IMAGE_EMBEDDING_MAX_BATCH_BYTES, 20 * 1024 * 1024);
    }
}
