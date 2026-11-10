use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use qdrant_client::{Qdrant, qdrant::PointId};
use std::time::Duration;
use uuid::Uuid;

mod cleanup;
pub(crate) mod code_vectors;
mod collection;
mod date_window;
mod errors;
pub mod metadata_retirement;
mod payload;
mod replacement;
mod search;
mod writes;

#[cfg(feature = "integration-test-helpers")]
mod test_helpers;

pub use errors::{format_qdrant_error, qdrant_timeout_error, truncate_for_qdrant_error};
pub use writes::is_qdrant_idempotent_not_found;

const QDRANT_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct SearchPointHit {
    pub chunk_id: Uuid,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct SearchDatePointHit {
    pub chunk_id: Uuid,
    /// Integer-second `published_ts` copied from the Qdrant payload. `None`
    /// when the chunk was indexed without a publication timestamp and must be
    /// treated as a stable (but out-of-band) tie-breaker.
    pub published_ts: Option<i64>,
    /// Vector similarity score from the Qdrant search; the date pipeline
    /// carries it only for hydration observability, never for sort order.
    pub score: f32,
}

#[derive(Clone)]
pub struct QdrantIndex {
    client: Qdrant,
    collection_name: String,
    endpoint: String,
    dimensions: usize,
    noop: bool,
}

/// The exact physical Qdrant target a logical delete committed against.
///
/// The endpoint is private application configuration: the `Debug` rendering
/// deliberately omits it so a log line cannot leak credentials embedded in the
/// URL.
#[derive(Clone, PartialEq, Eq)]
pub struct PurgeTarget {
    pub(crate) endpoint: String,
    pub(crate) collection: String,
}

impl std::fmt::Debug for PurgeTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PurgeTarget")
            .field("collection", &self.collection)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for QdrantIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QdrantIndex")
            .field("collection_name", &self.collection_name)
            .field("dimensions", &self.dimensions)
            .finish_non_exhaustive()
    }
}

impl QdrantIndex {
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    pub(crate) fn purge_target(&self) -> PurgeTarget {
        PurgeTarget {
            endpoint: self.endpoint.clone(),
            collection: self.collection_name.clone(),
        }
    }

    /// Bind a client to one recorded physical purge target.
    ///
    /// This constructor performs no network I/O and no collection
    /// create/ensure/upsert. It exists so a pending tombstone can retry the
    /// exact endpoint and collection named at deletion time, never the current
    /// desired settings.
    pub(crate) fn for_recorded_target(endpoint: &str, collection: &str) -> Result<Self> {
        let noop = {
            #[cfg(any(test, feature = "integration-test-helpers"))]
            {
                collection == "test-noop"
            }
            #[cfg(not(any(test, feature = "integration-test-helpers")))]
            {
                false
            }
        };
        Ok(Self {
            client: Qdrant::from_url(endpoint).build()?,
            collection_name: collection.to_string(),
            endpoint: endpoint.to_string(),
            dimensions: 0,
            noop,
        })
    }
}

fn point_id_to_uuid(point_id: PointId) -> Result<Uuid> {
    let raw = point_id
        .point_id_options
        .map(|value| match value {
            qdrant_client::qdrant::point_id::PointIdOptions::Uuid(value) => value,
            qdrant_client::qdrant::point_id::PointIdOptions::Num(value) => value.to_string(),
        })
        .context("unsupported point id")?;
    Ok(Uuid::parse_str(&raw)?)
}

fn date_to_timestamp(date: DateTime<Utc>) -> i64 {
    date.timestamp()
}
