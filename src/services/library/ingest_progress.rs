//! Bounded, content-free pipeline progress for one task item (issue 52).
//!
//! `pipeline_progress` is a sibling of `indexing_checkpoint` inside the
//! existing `task_items.payload` JSON, deliberately not a field of it: the
//! checkpoint rejects any write that does not advance `next_batch_index`, while
//! progress legitimately moves while a single batch is still embedding. It
//! carries no document text, no `section_payload` and no copy of the payload —
//! only the attempt it belongs to, the stage the item really reached, counters
//! measured by the same chunking run that produces the batches, an optional
//! remote converter status, and the time real progress was last observed.
//!
//! The stored encoding and its validation live in
//! `super::ingest_progress_codec`; the lease-conditional write in
//! `super::ingest_progress_store`.

use chrono::{DateTime, Utc};

use crate::contracts::TaskItemProgress;
use crate::services::library::UnifiedIngestError;

use super::task_ingest::task_failure;

/// Stable JSON key reserved for pipeline progress inside the task item payload.
pub const PIPELINE_PROGRESS_KEY: &str = "pipeline_progress";

/// Current progress schema version. A record with a different version is
/// treated as absent so an old format can never render as live progress.
pub const PIPELINE_PROGRESS_VERSION: u32 = 1;

/// Hard upper bound on the serialized size of one progress record. Progress
/// rides inside the payload every other ingest key shares, so it must stay a
/// fixed few hundred bytes regardless of document size.
pub const PIPELINE_PROGRESS_MAX_BYTES: usize = 1024;

/// The pipeline stage a progress record reports.
///
/// These are the stages an item *actually* performs, which is not the same set
/// as the persisted item stage column (that stays `processing` for the whole
/// run). Text ingestion never reports [`Self::Docling`]: it has no remote
/// conversion to wait for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineProgressStage {
    Storage,
    Parsing,
    Docling,
    Chunking,
    Indexing,
    Finalizing,
}

impl PipelineProgressStage {
    /// Every stage this build can report, in pipeline order.
    const ALL: [Self; 6] = [
        Self::Storage,
        Self::Parsing,
        Self::Docling,
        Self::Chunking,
        Self::Indexing,
        Self::Finalizing,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::Parsing => "parsing",
            Self::Docling => "docling",
            Self::Chunking => "chunking",
            Self::Indexing => "indexing",
            Self::Finalizing => "finalizing",
        }
    }

    /// Parse a persisted stage hint. An unknown value is rejected instead of
    /// being rendered as an unlabelled stage.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.as_str() == value)
    }
}

/// One observation of real progress for one attempt of one item.
///
/// Counters are optional because unknown stays unknown: an absent `total_*`
/// means the run has not published its totals, not that it has zero work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineProgress {
    pub attempt: i32,
    pub stage: PipelineProgressStage,
    pub completed_chunks: Option<u32>,
    pub total_chunks: Option<u32>,
    pub completed_batches: Option<u32>,
    pub total_batches: Option<u32>,
    pub remote_status: Option<String>,
    pub last_progress_at: Option<DateTime<Utc>>,
}

impl PipelineProgress {
    /// A fresh record for one attempt at one stage, with no counters yet.
    pub fn new(attempt: i32, stage: PipelineProgressStage) -> Self {
        Self {
            attempt,
            stage,
            completed_chunks: None,
            total_chunks: None,
            completed_batches: None,
            total_batches: None,
            remote_status: None,
            last_progress_at: None,
        }
    }

    /// Publish the totals this run will produce, measured by the same
    /// chunking pass that produces the batches. Completed counters stay
    /// absent: a resumed run already checkpointed batches whose chunk count is
    /// only recovered as the loop re-walks them, so publishing `0` here would
    /// be an untruthful observation rather than an unknown one.
    pub fn with_known_totals(
        self,
        total_chunks: usize,
        total_batches: usize,
    ) -> Result<Self, UnifiedIngestError> {
        Ok(self
            .with_batch_counters(0, total_chunks, 0, total_batches)?
            .without_completed())
    }

    /// Publish all four counters of one indexing batch boundary.
    ///
    /// The `usize` inputs come from the chunking loop and are bounded by the
    /// document; a value that no longer fits the record's counter width is a
    /// failure to report rather than a value to wrap.
    pub fn with_batch_counters(
        mut self,
        completed_chunks: usize,
        total_chunks: usize,
        completed_batches: usize,
        total_batches: usize,
    ) -> Result<Self, UnifiedIngestError> {
        self.completed_chunks = Some(bounded_count(completed_chunks, "completed chunk")?);
        self.total_chunks = Some(bounded_count(total_chunks, "total chunk")?);
        self.completed_batches = Some(bounded_count(completed_batches, "completed batch")?);
        self.total_batches = Some(bounded_count(total_batches, "total batch")?);
        Ok(self)
    }

    /// Stamp the record as a real observation made now.
    ///
    /// Only the writers call this, and only when work actually advanced, so
    /// the timestamp stays a progress signal rather than a heartbeat. An
    /// unchanged remote status is not work and must not be stamped.
    pub fn observed_now(mut self) -> Self {
        self.last_progress_at = Some(Utc::now());
        self
    }

    /// The public API projection of this record.
    pub fn as_response(&self) -> TaskItemProgress {
        TaskItemProgress {
            stage: self.stage.as_str().to_string(),
            attempt: self.attempt,
            completed_chunks: self.completed_chunks.map(i64::from),
            total_chunks: self.total_chunks.map(i64::from),
            completed_batches: self.completed_batches.map(i64::from),
            total_batches: self.total_batches.map(i64::from),
            remote_status: self.remote_status.clone(),
            last_progress_at: self.last_progress_at,
        }
    }

    fn without_completed(mut self) -> Self {
        self.completed_chunks = None;
        self.completed_batches = None;
        self
    }
}

fn bounded_count(value: usize, label: &str) -> Result<u32, UnifiedIngestError> {
    u32::try_from(value).map_err(|_| {
        task_failure(
            "indexing",
            anyhow::anyhow!("{label} count {value} exceeds the progress bound"),
            false,
        )
    })
}
