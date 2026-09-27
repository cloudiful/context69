use anyhow::{Context, Result};

use tracing::{error, warn};

use crate::{db::StoredMetadataIndex, domain_errors::DomainError};

use super::{DocumentStoreService, metadata};

/// Documents fetched per page while (re)building a metadata index. The build
/// walks the group/source documents by `id` cursor so a large library never
/// materializes every row (and its extracted values) at once.
const METADATA_INDEX_PAGE_SIZE: i64 = 500;

impl DocumentStoreService {
    pub(super) fn spawn_worker(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.run_pending().await {
                error!(error = %error, "metadata index worker failed");
            }
        });
    }

    async fn run_pending(&self) -> Result<()> {
        let Ok(_guard) = self.worker_lock.try_lock() else {
            return Ok(());
        };
        for definition in self.db.pending_metadata_indexes().await? {
            if definition.status == "deleting" {
                if let Some(index) = &self.index
                    && let Err(error) = index
                        .delete_metadata_field_index(&definition.field_path)
                        .await
                {
                    warn!(index_id = %definition.index_id, error = %error, "failed to delete qdrant metadata field index");
                }
                self.db.remove_metadata_index(definition.index_id).await?;
                continue;
            }
            if let Err(error) = self.build_index(&definition).await {
                self.db
                    .fail_metadata_index(definition.index_id, &error.to_string())
                    .await?;
            }
        }
        Ok(())
    }

    async fn build_index(&self, definition: &StoredMetadataIndex) -> Result<()> {
        let mut cursor = 0_i64;
        let mut processed = 0_i64;
        loop {
            let documents = self
                .db
                .metadata_documents_page(definition, cursor, METADATA_INDEX_PAGE_SIZE)
                .await?;
            let Some(last) = documents.last() else {
                break;
            };
            cursor = last.document_id;
            let mut metadata_keys = Vec::with_capacity(documents.len());
            let mut metadata_values = Vec::new();
            for document in documents {
                metadata_keys.push((definition.index_id, document.document_id));
                let values = metadata::extract_values(definition, &document.metadata_json)
                    .with_context(|| {
                        DomainError::internal(format!(
                            "document {} metadata field {}",
                            document.document_id, definition.field_path
                        ))
                    })?;
                metadata_values.extend(crate::db::metadata_value_rows(
                    definition.index_id,
                    document.document_id,
                    &values,
                ));
                processed += 1;
            }
            self.db
                .replace_metadata_values_bulk(&metadata_keys, &metadata_values)
                .await?;
        }
        if let Some(index) = &self.index {
            index
                .ensure_metadata_field_index(&definition.field_path, &definition.data_type)
                .await?;
        }
        self.db
            .finish_metadata_index(definition.index_id, processed)
            .await
    }
}
