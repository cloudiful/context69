use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use crate::{
    config::Config,
    db::Database,
    embedding::{EmbeddingProvider, OpenAiCompatibleEmbeddingProvider},
    qdrant_index::QdrantIndex,
};

pub struct VectorRuntime {
    pub embedding: Option<Arc<dyn EmbeddingProvider>>,
    pub index: Option<QdrantIndex>,
    pub embedding_vector_configured: bool,
    pub collection_needs_rebuild: bool,
    pub fingerprint: String,
    pub fingerprint_changed: bool,
}

pub async fn initialize(
    db: &Database,
    config: &Config,
    runtime_configured: bool,
) -> Result<VectorRuntime> {
    let fingerprint = super::vector_identity::fingerprint(config);
    let stored_fingerprint = db
        .get_vector_index_fingerprint(&config.qdrant.collection_name)
        .await?;
    let fingerprint_changed = stored_fingerprint.as_deref() != Some(&fingerprint);

    let mut embedding: Option<Arc<dyn EmbeddingProvider>> = None;
    let mut index: Option<QdrantIndex> = None;
    let mut embedding_vector_configured = false;
    let mut collection_needs_rebuild = false;

    if runtime_configured {
        match OpenAiCompatibleEmbeddingProvider::new(config.embedding.clone()) {
            Ok(provider) => {
                embedding_vector_configured = true;
                let provider: Arc<dyn EmbeddingProvider> = Arc::new(provider);
                let mut qdrant_config = config.qdrant.clone();
                qdrant_config.recreate_on_dimension_mismatch |= fingerprint_changed;
                match QdrantIndex::connect(&qdrant_config, config.embedding.dimensions).await {
                    Ok((connected_index, recreated)) => {
                        embedding = Some(provider);
                        index = Some(connected_index);
                        collection_needs_rebuild = recreated;
                    }
                    Err(error) => {
                        warn!(error = %error, "qdrant runtime is unavailable; continuing in degraded mode");
                    }
                }
            }
            Err(error) => {
                warn!(error = %error, "embedding runtime is unavailable; continuing in degraded mode");
            }
        }
    }

    Ok(VectorRuntime {
        embedding,
        index,
        embedding_vector_configured,
        collection_needs_rebuild,
        fingerprint,
        fingerprint_changed,
    })
}
