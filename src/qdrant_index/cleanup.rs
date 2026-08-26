use anyhow::Result;
use qdrant_client::qdrant::{Condition, DeletePointsBuilder, Filter};
use tokio::time::timeout;
use uuid::Uuid;

use super::{
    QDRANT_OPERATION_TIMEOUT, QdrantIndex, format_qdrant_error, is_qdrant_idempotent_not_found,
    qdrant_timeout_error,
};

impl QdrantIndex {
    pub(crate) async fn delete_points_for_library_file(&self, file_id: Uuid) -> Result<()> {
        let filter = Filter::must([Condition::matches("library_file_id", file_id.to_string())]);
        let operation = "delete_points_for_library_file";
        let collection = self.collection_name.clone();
        let extra = format!("file_id={file_id}");
        let result = timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.delete_points(
                DeletePointsBuilder::new(&self.collection_name)
                    .points(filter)
                    .wait(true),
            ),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, &extra));

        let result = match result {
            Err(err) => Err(err),
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => {
                let err_anyhow: anyhow::Error = err.into();
                if is_qdrant_idempotent_not_found(&err_anyhow) {
                    // Orphan point cleanup is idempotent: missing filter match is success.
                    return Ok(());
                }
                Err(format_qdrant_error(
                    operation,
                    &collection,
                    &extra,
                    err_anyhow,
                ))
            }
        };
        result?;
        Ok(())
    }
}
