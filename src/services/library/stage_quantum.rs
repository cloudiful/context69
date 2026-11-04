//! The finite work quantum shared by both ingest staging kernels.
//!
//! The configured value is snapshotted once per task indexing segment and then
//! drives the same predicate in the atomic and legacy paths: a segment yields
//! only after it committed its configured number of batches and only while more
//! batches remain. The direct non-task wrappers stay unlimited.

use anyhow::Result;

use crate::contracts::settings::{INGEST_BATCHES_PER_SEGMENT_MAX, INGEST_BATCHES_PER_SEGMENT_MIN};

/// Validates a finite task-segment quantum.
///
/// A zero or oversized value is a deterministic configuration error; it is
/// never clamped or turned into an unbounded segment.
pub(super) fn validate_finite_batches_per_segment(limit: usize) -> Result<usize> {
    if !(INGEST_BATCHES_PER_SEGMENT_MIN..=INGEST_BATCHES_PER_SEGMENT_MAX).contains(&limit) {
        return Err(crate::domain_errors::DomainError::invalid_argument(format!(
            "ingest batches per segment {limit} must be between {INGEST_BATCHES_PER_SEGMENT_MIN} \
             and {INGEST_BATCHES_PER_SEGMENT_MAX}"
        ))
        .into());
    }
    Ok(limit)
}

/// Whether a finite segment must yield after committing its quantum.
pub(super) fn finite_turn_should_yield(
    committed_this_segment: usize,
    limit: usize,
    next_batch_index: usize,
    total_batches: usize,
) -> bool {
    committed_this_segment >= limit && next_batch_index < total_batches
}
