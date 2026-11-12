//! Deleting one recorded physical document collection.
//!
//! The retirement worker binds a client to the endpoint and name captured when
//! the collection was retired, never to the current settings. A missing
//! collection is an idempotent success; every other Qdrant failure replays the
//! standard operation timeout and error mapping.

use anyhow::Result;
use qdrant_client::qdrant::DeleteCollectionBuilder;
use tokio::time::timeout;

use super::errors::bounded_qdrant_chain;
use super::{QDRANT_OPERATION_TIMEOUT, QdrantIndex, format_qdrant_error, qdrant_timeout_error};

/// The observed result of one collection delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionDeletion {
    /// The request removed the collection.
    Deleted,
    /// Qdrant already reports the collection as absent; deletion is complete.
    AlreadyAbsent,
}

/// Whether an error reports exactly an absent collection.
///
/// Only an explicit not-found collection signal qualifies. Permission,
/// authentication, validation, and transport failures are never treated as an
/// absent collection, so a real deletion failure stays pending and retries.
pub(crate) fn collection_absence(error: &anyhow::Error) -> bool {
    if let Some(typed) = crate::domain_errors::find_domain_error(error)
        && matches!(
            typed,
            crate::domain_errors::DomainError::Forbidden(_)
                | crate::domain_errors::DomainError::Unauthorized(_)
                | crate::domain_errors::DomainError::InvalidArgument(_)
                | crate::domain_errors::DomainError::UnprocessableEntity(_)
                | crate::domain_errors::DomainError::Conflict(_)
                | crate::domain_errors::DomainError::PayloadTooLarge(_)
        )
    {
        return false;
    }
    let text = bounded_qdrant_chain(error).to_ascii_lowercase();
    let not_found = text.contains("not found") || text.contains("notfound");
    if !not_found || !text.contains("collection") {
        return false;
    }
    !(text.contains("permission")
        || text.contains("unauthorized")
        || text.contains("unauthenticated")
        || text.contains("forbidden")
        || text.contains("authentication")
        || text.contains("validation")
        || text.contains("invalid_argument")
        || text.contains("unsupported"))
}

impl QdrantIndex {
    /// Delete the collection this index is bound to.
    ///
    /// The caller has already proven that this endpoint owns the name and that
    /// no active or recorded rebuild target still uses it.
    pub(crate) async fn delete_recorded_collection(&self) -> Result<CollectionDeletion> {
        if self.is_noop() {
            return Ok(CollectionDeletion::AlreadyAbsent);
        }
        let operation = "delete_collection";
        let collection = self.collection_name.clone();
        let result = timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client
                .delete_collection(DeleteCollectionBuilder::new(&self.collection_name).build()),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, ""));
        match result {
            Err(error) => Err(error),
            Ok(Ok(_)) => Ok(CollectionDeletion::Deleted),
            Ok(Err(error)) => {
                let error: anyhow::Error = error.into();
                if collection_absence(&error) {
                    Ok(CollectionDeletion::AlreadyAbsent)
                } else {
                    Err(format_qdrant_error(operation, &collection, "", error))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collection_absence;

    #[test]
    fn only_an_absent_collection_is_idempotent_success() {
        assert!(collection_absence(&anyhow::anyhow!(
            "qdrant error: Not found: Collection `context69_chunks` doesn't exist | code: NotFound"
        )));
        assert!(!collection_absence(&anyhow::anyhow!(
            "qdrant error: permission denied for collection"
        )));
        assert!(!collection_absence(&anyhow::anyhow!(
            "qdrant error: validation error: collection name is invalid"
        )));
        assert!(!collection_absence(&anyhow::anyhow!(
            "qdrant error: point id not found"
        )));
        assert!(!collection_absence(&anyhow::anyhow!(
            "qdrant error: transport connect failed"
        )));
    }
}
