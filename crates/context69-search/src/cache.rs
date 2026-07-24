use std::{sync::Arc, time::Duration};

use anyhow::Result;
use context69_contracts::{SearchRequest, SearchResponse};
use redis::{AsyncCommands, Client, RedisError, aio::ConnectionManager, cmd};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;
use uuid::Uuid;

use crate::{RerankHit, SearchSettings};

const SEARCH_KEY_PREFIX: &str = "context69:search:v1:";
const EMBED_TTL_SECS: u64 = 24 * 60 * 60;
const RERANK_ITEM_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const RERANK_BATCH_TTL_SECS: u64 = 15 * 60;
const RESPONSE_TTL_SECS: u64 = 2 * 60;
const EMPTY_RESPONSE_TTL_SECS: u64 = 60;

#[derive(Clone)]
pub struct SearchCache {
    connection: Option<Arc<ConnectionManager>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRerankItemScore {
    pub chunk_id: Uuid,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedEmbedding {
    vector: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedRerankBatch {
    hits: Vec<RerankHit>,
}

impl SearchCache {
    pub async fn new(valkey_url: Option<&str>) -> Self {
        let Some(url) = valkey_url.filter(|value| !value.trim().is_empty()) else {
            return Self { connection: None };
        };

        let client = match Client::open(url) {
            Ok(client) => client,
            Err(error) => {
                warn!(error = %error, "search cache valkey configuration invalid; continuing without online search cache");
                return Self { connection: None };
            }
        };

        let connection = match client.get_connection_manager().await {
            Ok(connection) => connection,
            Err(error) => {
                warn!(error = %error, "search cache valkey unavailable; continuing without online search cache");
                return Self { connection: None };
            }
        };

        Self {
            connection: Some(Arc::new(connection)),
        }
    }

    pub fn query_hash(query: &str) -> String {
        hash_string(query.trim())
    }

    pub fn request_hash(request: &SearchRequest) -> String {
        hash_string(&serde_json::to_string(request).unwrap_or_default())
    }

    pub fn settings_hash(settings: &SearchSettings) -> String {
        hash_string(&format!(
            "mode={}\nrerank_enabled={}\nrerank_model={}\ncandidate_limit={}",
            settings.mode.as_str(),
            settings.rerank_enabled,
            settings.rerank_model,
            settings.candidate_limit
        ))
    }

    pub fn candidate_hash(chunk_ids: &[Uuid]) -> String {
        hash_string(
            &chunk_ids
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(","),
        )
    }

    pub async fn get_query_embedding(&self, model: &str, query_hash: &str) -> Option<Vec<f32>> {
        let key = format!("{SEARCH_KEY_PREFIX}embed:{model}:{query_hash}");
        self.get_json::<CachedEmbedding>(&key)
            .await
            .map(|value| value.vector)
    }

    pub async fn set_query_embedding(&self, model: &str, query_hash: &str, vector: &[f32]) {
        let key = format!("{SEARCH_KEY_PREFIX}embed:{model}:{query_hash}");
        self.set_json(
            &key,
            &CachedEmbedding {
                vector: vector.to_vec(),
            },
            Duration::from_secs(EMBED_TTL_SECS),
        )
        .await;
    }

    pub async fn get_rerank_batch(
        &self,
        generation: i64,
        rerank_model: &str,
        query_hash: &str,
        top_n: usize,
        candidate_hash: &str,
    ) -> Option<Vec<RerankHit>> {
        let key = format!(
            "{SEARCH_KEY_PREFIX}rerank_batch:g{generation}:{rerank_model}:{query_hash}:{top_n}:{candidate_hash}"
        );
        self.get_json::<CachedRerankBatch>(&key)
            .await
            .map(|value| value.hits)
    }

    pub async fn set_rerank_batch(
        &self,
        generation: i64,
        rerank_model: &str,
        query_hash: &str,
        top_n: usize,
        candidate_hash: &str,
        hits: &[RerankHit],
    ) {
        let key = format!(
            "{SEARCH_KEY_PREFIX}rerank_batch:g{generation}:{rerank_model}:{query_hash}:{top_n}:{candidate_hash}"
        );
        self.set_json(
            &key,
            &CachedRerankBatch {
                hits: hits.to_vec(),
            },
            Duration::from_secs(RERANK_BATCH_TTL_SECS),
        )
        .await;
    }

    pub async fn get_rerank_item_scores(
        &self,
        rerank_model: &str,
        query_hash: &str,
        chunk_ids: &[Uuid],
    ) -> Vec<CachedRerankItemScore> {
        let Some(connection) = &self.connection else {
            return Vec::new();
        };
        if chunk_ids.is_empty() {
            return Vec::new();
        }

        let keys = chunk_ids
            .iter()
            .map(|chunk_id| {
                format!("{SEARCH_KEY_PREFIX}rerank_item:{rerank_model}:{query_hash}:{chunk_id}")
            })
            .collect::<Vec<_>>();

        let mut connection = (**connection).clone();
        let values: Result<Vec<Option<String>>, RedisError> =
            cmd("MGET").arg(keys).query_async(&mut connection).await;

        match values {
            Ok(values) => values
                .into_iter()
                .filter_map(|value| {
                    value.and_then(|payload| {
                        serde_json::from_str::<CachedRerankItemScore>(&payload).ok()
                    })
                })
                .collect(),
            Err(error) => {
                warn!(error = %error, "search cache rerank item read failed; falling back");
                Vec::new()
            }
        }
    }

    pub async fn set_rerank_item_scores(
        &self,
        rerank_model: &str,
        query_hash: &str,
        scores: &[CachedRerankItemScore],
    ) {
        let Some(connection) = &self.connection else {
            return;
        };
        if scores.is_empty() {
            return;
        }

        let mut connection = (**connection).clone();
        let mut pipeline = redis::pipe();
        pipeline.atomic();
        for score in scores {
            let key = format!(
                "{SEARCH_KEY_PREFIX}rerank_item:{rerank_model}:{query_hash}:{}",
                score.chunk_id
            );
            let Ok(payload) = serde_json::to_string(score) else {
                continue;
            };
            pipeline
                .cmd("SET")
                .arg(key)
                .arg(payload)
                .arg("EX")
                .arg(RERANK_ITEM_TTL_SECS);
        }

        if let Err(error) = pipeline.query_async::<()>(&mut connection).await {
            warn!(error = %error, "search cache rerank item write failed; continuing");
        }
    }

    pub async fn get_search_response(
        &self,
        generation: i64,
        request_hash: &str,
        settings_hash: &str,
    ) -> Option<SearchResponse> {
        let key =
            format!("{SEARCH_KEY_PREFIX}response:g{generation}:{request_hash}:{settings_hash}");
        self.get_json::<SearchResponse>(&key).await
    }

    pub async fn set_search_response(
        &self,
        generation: i64,
        request_hash: &str,
        settings_hash: &str,
        response: &SearchResponse,
    ) {
        let key =
            format!("{SEARCH_KEY_PREFIX}response:g{generation}:{request_hash}:{settings_hash}");
        let ttl = if response.items.is_empty() {
            Duration::from_secs(EMPTY_RESPONSE_TTL_SECS)
        } else {
            Duration::from_secs(RESPONSE_TTL_SECS)
        };
        self.set_json(&key, response, ttl).await;
    }

    async fn get_json<T>(&self, key: &str) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let Some(connection) = &self.connection else {
            return None;
        };
        let mut connection = (**connection).clone();
        match connection.get::<_, Option<String>>(key).await {
            Ok(Some(payload)) => {
                let parsed = serde_json::from_str::<T>(&payload);
                match parsed {
                    Ok(value) => Some(value),
                    Err(error) => {
                        warn!(error = %error, key, "search cache decode failed; ignoring cached value");
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(error) => {
                warn!(error = %error, key, "search cache read failed; falling back");
                None
            }
        }
    }

    async fn set_json<T>(&self, key: &str, value: &T, ttl: Duration)
    where
        T: Serialize,
    {
        let Some(connection) = &self.connection else {
            return;
        };
        let Ok(payload) = serde_json::to_string(value) else {
            return;
        };
        let mut connection = (**connection).clone();
        if let Err(error) = connection
            .set_ex::<_, _, ()>(key, payload, ttl.as_secs())
            .await
        {
            warn!(error = %error, key, "search cache write failed; continuing");
        }
    }
}

pub fn hash_string(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex_string(&hasher.finalize())
}

fn hex_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub fn rerank_hits_from_item_scores(scores: &[CachedRerankItemScore]) -> Vec<RerankHit> {
    let mut indexed = scores
        .iter()
        .enumerate()
        .map(|(index, score)| RerankHit {
            index,
            score: score.score,
        })
        .collect::<Vec<_>>();
    indexed.sort_by(|left, right| right.score.total_cmp(&left.score));
    indexed
}

pub fn rerank_item_scores_from_hits(
    chunk_ids: &[Uuid],
    hits: &[RerankHit],
) -> Vec<CachedRerankItemScore> {
    hits.iter()
        .filter_map(|hit| {
            chunk_ids
                .get(hit.index)
                .copied()
                .map(|chunk_id| CachedRerankItemScore {
                    chunk_id,
                    score: hit.score,
                })
        })
        .collect()
}

pub fn rerank_item_scores_complete(
    expected_chunk_ids: &[Uuid],
    cached_scores: &[CachedRerankItemScore],
) -> bool {
    expected_chunk_ids.len() == cached_scores.len()
        && expected_chunk_ids.iter().all(|chunk_id| {
            cached_scores
                .iter()
                .any(|cached| cached.chunk_id == *chunk_id)
        })
}

pub fn sort_cached_item_scores(
    expected_chunk_ids: &[Uuid],
    cached_scores: &[CachedRerankItemScore],
) -> Vec<CachedRerankItemScore> {
    expected_chunk_ids
        .iter()
        .filter_map(|chunk_id| {
            cached_scores
                .iter()
                .find(|cached| cached.chunk_id == *chunk_id)
                .cloned()
        })
        .collect()
}
