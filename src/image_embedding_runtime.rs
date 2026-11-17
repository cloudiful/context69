//! The process-wide handle to the optional image embedding provider.
//!
//! Image embedding is independent of the document and code embedding runtimes:
//! it has its own configuration, credential, immutable snapshot, and physical
//! Qdrant collection, and it never borrows a document value. The handle
//! publishes one complete snapshot at a time, so a reader clones either the
//! previous or the next provider/config/fingerprint triple and can never mix
//! fields from two installations.
//!
//! A task item that starts after an install keeps the snapshot it captured;
//! changing image settings never relabels vectors an in-flight item is already
//! producing.

use std::sync::{Arc, RwLock};

use tokio::sync::{RwLock as AsyncRwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{
    config::ImageEmbeddingConfig,
    image_embedding::{ImageEmbeddingProvider, image_embedding_fingerprint},
    image_embedding_vectors::ImageVectorTarget,
};

/// One immutable image embedding provider, its configuration, and its identity.
#[derive(Clone)]
pub struct ImageEmbeddingSnapshot {
    pub provider: Arc<dyn ImageEmbeddingProvider>,
    pub config: ImageEmbeddingConfig,
    pub fingerprint: String,
    /// The complete physical destination (endpoint, image collection, and
    /// provider fingerprint) in effect with this snapshot. It is captured and
    /// replaced together with the provider, so a final publisher never rebuilds
    /// it from mutable settings.
    pub destination: Option<ImageVectorTarget>,
}

/// A shared handle to the image embedding snapshot currently in effect.
#[derive(Clone, Default)]
pub struct ImageEmbeddingRuntime {
    snapshot: Arc<RwLock<Option<Arc<ImageEmbeddingSnapshot>>>>,
    /// Serializes a committed install/disable with the final bounded image
    /// publication. The settings update takes the write side; a publication
    /// takes the read side only for its short Qdrant call and acknowledgement,
    /// never for provider embedding or collection preparation.
    publication: Arc<AsyncRwLock<()>>,
}

impl ImageEmbeddingRuntime {
    /// An unconfigured handle. Image embedding is disabled by default.
    pub fn new() -> Self {
        Self::default()
    }

    /// The snapshot currently in effect, or `None` when image embedding is not
    /// configured.
    pub fn snapshot(&self) -> Option<Arc<ImageEmbeddingSnapshot>> {
        self.read().clone()
    }

    /// The write side of the publication guard.
    ///
    /// A settings save takes this before it commits, so a publication that is
    /// admitted later observes the installed or disabled snapshot.
    pub async fn publication_write(&self) -> RwLockWriteGuard<'_, ()> {
        self.publication.write().await
    }

    /// The read side of the publication guard.
    ///
    /// The caller holds it across the final short Qdrant call and
    /// acknowledgement, and re-reads [`Self::snapshot`] after acquiring it.
    pub async fn publication_read(&self) -> RwLockReadGuard<'_, ()> {
        self.publication.read().await
    }

    /// Atomically installs a new provider, configuration, and identity.
    pub async fn install(
        &self,
        provider: Arc<dyn ImageEmbeddingProvider>,
        config: ImageEmbeddingConfig,
        fingerprint: String,
        destination: Option<ImageVectorTarget>,
    ) {
        let _guard = self.publication_write().await;
        self.install_locked(provider, config, fingerprint, destination);
    }

    /// Installs a snapshot while the caller already holds the publication
    /// write guard.
    pub fn install_locked(
        &self,
        provider: Arc<dyn ImageEmbeddingProvider>,
        config: ImageEmbeddingConfig,
        fingerprint: String,
        destination: Option<ImageVectorTarget>,
    ) {
        *self.write() = Some(Arc::new(ImageEmbeddingSnapshot {
            provider,
            config,
            fingerprint,
            destination,
        }));
    }

    /// Atomically removes the current snapshot, leaving image embedding
    /// unconfigured. An item that already captured its snapshot keeps it.
    pub async fn disable(&self) {
        let _guard = self.publication_write().await;
        self.disable_locked();
    }

    /// Disables the channel while the caller already holds the publication
    /// write guard.
    pub fn disable_locked(&self) {
        *self.write() = None;
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Option<Arc<ImageEmbeddingSnapshot>>> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Option<Arc<ImageEmbeddingSnapshot>>> {
        self.snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The fingerprint of one image configuration.
pub fn image_fingerprint(config: &ImageEmbeddingConfig) -> String {
    image_embedding_fingerprint(config)
}

#[cfg(test)]
mod tests {
    use super::ImageEmbeddingRuntime;

    #[tokio::test]
    async fn an_unconfigured_runtime_has_no_snapshot_and_disable_keeps_it_empty() {
        let runtime = ImageEmbeddingRuntime::new();
        assert!(runtime.snapshot().is_none());
        runtime.disable().await;
        assert!(runtime.snapshot().is_none());
    }

    #[test]
    fn the_default_config_is_disabled_by_absence() {
        assert!(crate::config::Config::default().image_embedding.is_none());
    }
}
