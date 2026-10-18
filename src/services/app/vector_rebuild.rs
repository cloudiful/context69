use std::sync::Arc;

use anyhow::Result;
use tracing::{error, info};

use crate::{
    config::Config,
    db::{Database, VectorIndexState},
    qdrant_index::QdrantIndex,
    services::sync::SyncService,
    vector_runtime::VectorRuntime,
};

/// Completion hook for one explicit vector-index rebuild.
///
/// The rebuild fence itself is committed by `VectorRuntime::begin_rebuild`; this
/// ticket only reports success once `complete_rebuild` kept that generation.
#[derive(Clone)]
pub struct VectorRebuildTicket {
    on_settled: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl VectorRebuildTicket {
    pub fn new() -> Self {
        Self { on_settled: None }
    }

    /// Registers a hook invoked once this rebuild reopens reads, so the
    /// dependency gates can be refreshed back to available.
    pub fn with_on_settled(mut self, on_settled: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.on_settled = Some(on_settled);
        self
    }

    /// Notifies the settle hook. Called only after a successful rebuild whose
    /// generation was still current.
    pub(crate) fn settle(&self) {
        if let Some(on_settled) = &self.on_settled {
            on_settled();
        }
    }
}

impl Default for VectorRebuildTicket {
    fn default() -> Self {
        Self::new()
    }
}

/// The shared runtime, its published index, and the generation a rebuild
/// started under.
pub struct RebuildTarget {
    pub runtime: VectorRuntime,
    pub index: Arc<QdrantIndex>,
    pub generation: u64,
}

pub fn spawn(
    sync: SyncService,
    db: Database,
    target: RebuildTarget,
    config: Config,
    fingerprint: String,
    recreate_collection: bool,
    ticket: VectorRebuildTicket,
) {
    let RebuildTarget {
        runtime,
        index,
        generation,
    } = target;
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
            // A settings change or a second rebuild may have started after this
            // one; only the still-current generation may reopen readiness.
            if runtime.complete_rebuild(generation) {
                ticket.settle();
            }
        } else {
            runtime.fail_rebuild(generation);
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::VectorRebuildTicket;

    #[test]
    fn a_ticket_notifies_its_settle_hook_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let hook: Arc<dyn Fn() + Send + Sync> = {
            let calls = calls.clone();
            Arc::new(move || {
                calls.fetch_add(1, Ordering::SeqCst);
            })
        };

        let ticket = VectorRebuildTicket::new().with_on_settled(hook);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        ticket.settle();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
