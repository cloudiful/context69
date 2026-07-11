use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use tracing::{error, info};

use crate::{
    config::Config,
    db::{Database, VectorIndexState},
    qdrant_index::QdrantIndex,
    services::sync::SyncService,
};

pub fn spawn(
    sync: SyncService,
    db: Database,
    index: QdrantIndex,
    config: Config,
    fingerprint: String,
    recreate_collection: bool,
    vector_index_ready: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        info!(
            collection_name = config.qdrant.collection_name,
            "automatic vector index rebuild started"
        );
        let result = rebuild(
            &sync,
            &db,
            &index,
            &config,
            &fingerprint,
            recreate_collection,
        )
        .await;
        if result.is_ok() {
            vector_index_ready.store(true, Ordering::Release);
        }
        if let Err(error) = &result {
            error!(error = %error, "automatic vector index rebuild failed");
        }
        sync.finish_vector_index_rebuild(result).await;
    });
}

async fn rebuild(
    sync: &SyncService,
    db: &Database,
    index: &QdrantIndex,
    config: &Config,
    fingerprint: &str,
    recreate_collection: bool,
) -> Result<usize> {
    if recreate_collection {
        index.recreate_collection().await?;
    }
    let rebuilt_chunks = sync.rebuild_index_from_db().await?;
    db.save_vector_index_state(&VectorIndexState {
        collection_name: &config.qdrant.collection_name,
        fingerprint,
        embedding_base_url: &config.embedding.base_url,
        embedding_model: &config.embedding.model,
        dimensions: config.embedding.dimensions,
        rebuilt_chunks,
    })
    .await?;
    Ok(rebuilt_chunks)
}
