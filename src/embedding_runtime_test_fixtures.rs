//! Fixtures shared by the embedding runtime's test modules.
//!
//! A provider that records how many times it was used, one that parks inside a
//! request until the test releases it, and the identity builder the assertions
//! compare against. Only the fixtures live here; every test stays in the module
//! for the contract it covers.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Notify;

use super::EmbeddingIdentity;
use crate::embedding::EmbeddingProvider;

/// A provider that records how many times it was used and returns a fixed
/// vector, so a test can tell which provider served a request.
#[derive(Default)]
pub(super) struct CountingProvider {
    pub(super) width: usize,
    pub(super) calls: Arc<AtomicUsize>,
}

impl CountingProvider {
    pub(super) fn new(width: usize) -> Self {
        Self {
            width,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EmbeddingProvider for CountingProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![0.0_f32; self.width]).collect())
    }
}

pub(super) fn identity(model: &str, dimensions: usize) -> EmbeddingIdentity {
    EmbeddingIdentity {
        base_url: "https://embeddings.example/v1".to_string(),
        model: model.to_string(),
        dimensions,
    }
}

/// A provider whose request parks until the test releases it, so a swap can be
/// observed while a request is still inside its network I/O.
pub(super) struct BlockingProvider {
    pub(super) width: usize,
    pub(super) started: Arc<Notify>,
    pub(super) release: Arc<Notify>,
}

#[async_trait]
impl EmbeddingProvider for BlockingProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(texts.iter().map(|_| vec![0.0_f32; self.width]).collect())
    }
}
