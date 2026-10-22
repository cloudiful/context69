use anyhow::Result;
use qdrant_client::qdrant::{Condition, DeletePointsBuilder, Filter};
use tokio::time::timeout;
use uuid::Uuid;

use super::{
    QDRANT_OPERATION_TIMEOUT, QdrantIndex, format_qdrant_error, is_qdrant_idempotent_not_found,
    qdrant_timeout_error,
};

/// Whether the cleanup of a no-op test index may skip the client.
///
/// The test lane is compiled only for a test build or the
/// `integration-test-helpers` feature. A production build has no bypass at all:
/// the function returns `false` for every index, whatever its collection is
/// named, so the cleanup behavior is exactly the one before the test lane
/// existed. No environment switch, configuration value or public API can turn
/// the bypass on in production.
#[cfg(any(test, feature = "integration-test-helpers"))]
fn cleanup_no_op(index: &QdrantIndex) -> bool {
    index.is_noop()
}

#[cfg(not(any(test, feature = "integration-test-helpers")))]
fn cleanup_no_op(_index: &QdrantIndex) -> bool {
    false
}

impl QdrantIndex {
    pub(crate) async fn delete_points_for_library_file(&self, file_id: Uuid) -> Result<()> {
        if cleanup_no_op(self) {
            return Ok(());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The test lane is available to a test build, so the whole-ingest proof
    /// can run without a reachable Qdrant.
    #[test]
    fn a_test_build_has_the_cleanup_test_lane() {
        assert!(cleanup_no_op(&QdrantIndex::for_test("test-noop", 4)));
        assert!(!cleanup_no_op(&QdrantIndex::for_test("library", 4)));
    }

    /// The production build of the gate is inert: it answers `false` for every
    /// index, so the reserved collection name cannot open a bypass outside the
    /// test lane.
    #[test]
    fn the_production_variant_of_the_gate_is_inert() {
        let source = include_str!("cleanup.rs");
        assert!(
            source.contains("#[cfg(not(any(test, feature = \"integration-test-helpers\")))]\nfn cleanup_no_op(_index: &QdrantIndex) -> bool {\n    false\n}"),
            "the production gate must be a compiled no-bypass variant"
        );
    }
}
