use anyhow::{Context, Result, anyhow};
use qdrant_client::qdrant::{Condition, DeletePointsBuilder, Filter};
use tokio::time::timeout;
use uuid::Uuid;

use super::{QDRANT_OPERATION_TIMEOUT, QdrantIndex};

impl QdrantIndex {
    pub(crate) async fn delete_points_for_library_file(&self, file_id: Uuid) -> Result<()> {
        let filter = Filter::must([Condition::matches("library_file_id", file_id.to_string())]);
        timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.delete_points(
                DeletePointsBuilder::new(&self.collection_name)
                    .points(filter)
                    .wait(true),
            ),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "qdrant library file cleanup request timed out after {}s",
                QDRANT_OPERATION_TIMEOUT.as_secs()
            )
        })?
        .context("qdrant library file cleanup request failed")?;
        Ok(())
    }
}
