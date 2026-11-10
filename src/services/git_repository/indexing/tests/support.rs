//! Pausing embedding and vector sinks shared by the vector tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Notify;

use crate::embedding::EmbeddingProvider;
use crate::qdrant_index::code_vectors::CodeVectorPoint;
use crate::services::git_repository::code_vectors::CodeVectorSink;

pub(super) struct PausingProvider {
    pub(super) entered: Notify,
    pub(super) resume: Notify,
    first_call: AtomicBool,
    pub(super) calls: AtomicUsize,
}

impl PausingProvider {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            resume: Notify::new(),
            first_call: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for PausingProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.first_call.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        Ok(texts.iter().map(|_| vec![0.25_f32; 8]).collect())
    }
}

pub(super) struct FailAfterFirstWriteSink {
    writes: AtomicUsize,
}

impl FailAfterFirstWriteSink {
    pub(super) fn new() -> Self {
        Self {
            writes: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl CodeVectorSink for FailAfterFirstWriteSink {
    async fn write(&self, _dimensions: usize, _points: &[CodeVectorPoint]) -> Result<()> {
        if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(());
        }
        Err(anyhow::anyhow!("git_code_sink_failed_after_partial"))
    }
}

pub(super) struct PausingSink {
    pub(super) entered: Notify,
    pub(super) resume: Notify,
    first_write: AtomicBool,
    pub(super) writes: AtomicUsize,
    pub(super) points: std::sync::Mutex<Vec<CodeVectorPoint>>,
}

impl PausingSink {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            resume: Notify::new(),
            first_write: AtomicBool::new(true),
            writes: AtomicUsize::new(0),
            points: std::sync::Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl CodeVectorSink for PausingSink {
    async fn write(&self, _dimensions: usize, points: &[CodeVectorPoint]) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.first_write.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        self.points.lock().unwrap().extend(points.iter().cloned());
        Ok(())
    }
}
