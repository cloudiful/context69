pub mod postgres_sql;

use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{SourceRecord, SyncCheckpoint};

#[async_trait]
pub trait SourceConnector: Send + Sync {
    async fn validate(&self) -> Result<()>;
    async fn fetch_batch(&self, checkpoint: &SyncCheckpoint) -> Result<Vec<SourceRecord>>;
}
