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

/// Completion handle for one explicit vector-index rebuild.
///
/// The ticket marks the index unready while the rebuild runs and reopens reads
/// once it succeeds. Only the explicit startup and manual rebuild paths create
/// one; a settings save never does, because an identity-changing save is
/// rejected before it is persisted.
#[derive(Clone)]
pub struct VectorRebuildTicket {
    readiness: Arc<AtomicBool>,
    on_settled: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl VectorRebuildTicket {
    pub fn new(readiness: Arc<AtomicBool>) -> Self {
        readiness.store(false, Ordering::Release);
        Self {
            readiness,
            on_settled: None,
        }
    }

    /// Registers a hook invoked once this rebuild reopens reads, so the
    /// dependency gates can be refreshed back to available.
    pub fn with_on_settled(mut self, on_settled: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.on_settled = Some(on_settled);
        self
    }

    /// Reopens reads and notifies the settle hook. Called only after a
    /// successful rebuild.
    pub(crate) fn settle(&self) {
        self.readiness.store(true, Ordering::Release);
        if let Some(on_settled) = &self.on_settled {
            on_settled();
        }
    }
}

pub fn spawn(
    sync: SyncService,
    db: Database,
    index: QdrantIndex,
    config: Config,
    fingerprint: String,
    recreate_collection: bool,
    ticket: VectorRebuildTicket,
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
            ticket.settle();
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
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use super::VectorRebuildTicket;

    #[test]
    fn a_ticket_marks_unready_then_reopens_reads_and_notifies() {
        let ready = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(AtomicUsize::new(0));
        let hook: Arc<dyn Fn() + Send + Sync> = {
            let calls = calls.clone();
            Arc::new(move || {
                calls.fetch_add(1, Ordering::SeqCst);
            })
        };

        let ticket = VectorRebuildTicket::new(ready.clone()).with_on_settled(hook);
        assert!(!ready.load(Ordering::Acquire));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        ticket.settle();
        assert!(ready.load(Ordering::Acquire));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
