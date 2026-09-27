use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use qdrant_client::{Qdrant, qdrant::PointId};
use std::time::Duration;
use uuid::Uuid;

mod cleanup;
mod collection;
mod date_window;
mod errors;
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
    dimensions: usize,
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
