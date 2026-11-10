//! The provider-side request planner for embedding batches.
//!
//! Every embedding request that leaves this deployment is planned here, so a
//! caller cannot choose its own grouping: the planner splits one input array
//! into the provider requests that carry it, in order, without touching the
//! texts themselves. The result is the same number of vectors in the same
//! order as the inputs, whatever grouping the planner chose.
//!
//! Two limits bound one automatic request:
//!
//! - the number of inputs it carries;
//! - the number of serialized bytes it occupies.
//!
//! A storage page size is not one of these limits. A caller that reads a page
//! of rows still sends its texts through this planner.
//!
//! The byte budget is measured against the exact serialized request, so a
//! request that would exceed it is split before it is sent rather than after.

use anyhow::Result;
use serde::Serialize;

use crate::domain_errors::DomainError;

/// The number of inputs one automatic embedding request may carry.
const EMBEDDING_REQUEST_MAX_ITEMS: usize = 32;
/// The number of serialized bytes one automatic embedding request may carry.
const EMBEDDING_REQUEST_MAX_BYTES: usize = 1_048_576;

/// The fixed transport limits one embedding provider request is bound by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingBatchLimits {
    /// The maximum number of inputs one request may carry.
    pub max_items_per_request: usize,
    /// The maximum number of serialized bytes one request may carry.
    pub max_request_bytes: usize,
}

impl EmbeddingBatchLimits {
    /// The fixed automatic transport packing of this deployment.
    pub fn fixed() -> Self {
        Self {
            max_items_per_request: EMBEDDING_REQUEST_MAX_ITEMS,
            max_request_bytes: EMBEDDING_REQUEST_MAX_BYTES,
        }
    }

    /// Whether every limit is usable. A zero limit would make every request
    /// impossible, so it is refused instead of planned.
    pub fn is_positive(&self) -> bool {
        self.max_items_per_request > 0 && self.max_request_bytes > 0
    }
}

/// One planned provider request: the half-open range of the input array it
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingBatch {
    pub start: usize,
    pub end: usize,
}

impl EmbeddingBatch {
    /// The inputs this batch carries.
    pub fn texts<'a>(&self, texts: &'a [String]) -> &'a [String] {
        &texts[self.start..self.end]
    }

    /// The number of inputs this batch carries.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether this batch carries no input.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// The request one provider call sends.
///
/// It lives here because the planner measures the exact serialized size of this
/// shape: the byte budget is only truthful if it is measured against the same
/// bytes that are sent.
#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: Vec<String>,
}

/// The number of bytes one request occupies when it carries `texts`.
///
/// The measurement is exact: it is the length of the serialized request, so a
/// planned batch never exceeds the byte budget it was planned against.
pub fn serialized_request_bytes(model: &str, texts: &[String]) -> usize {
    let request = EmbeddingRequest {
        model: model.to_string(),
        input: texts.to_vec(),
    };
    serde_json::to_string(&request)
        .map(|body| body.len())
        .unwrap_or(0)
}

/// The number of bytes the request envelope occupies.
///
/// The empty request is serialized once, so the envelope is measured the same
/// way the whole request is. It includes both JSON array brackets: a request
/// with inputs keeps them around the comma-separated items, so leaving them out
/// would undercount every request by two bytes.
fn envelope_bytes(model: &str) -> usize {
    let request = EmbeddingRequest {
        model: model.to_string(),
        input: Vec::new(),
    };
    serde_json::to_string(&request)
        .map(|body| body.len())
        .unwrap_or(0)
}

/// The number of bytes one input occupies inside a request.
fn item_bytes(text: &str) -> usize {
    serde_json::to_string(text)
        .map(|body| body.len())
        .unwrap_or(0)
}

/// Splits `texts` into the provider requests that carry them.
///
/// The inputs keep their order and their text: a batch is a range of the input
/// array, so a caller can join the returned vectors with the inputs without
/// reordering either.
///
/// An input that does not fit into one provider request is refused with an
/// actionable transport error. It is never split, shortened, or averaged,
/// because a shortened input would silently change the vector it produces.
pub fn plan_batches(
    texts: &[String],
    model: &str,
    limits: &EmbeddingBatchLimits,
) -> Result<Vec<EmbeddingBatch>> {
    if !limits.is_positive() {
        return Err(DomainError::invalid_argument(
            "the embedding transport limits must all be greater than 0",
        )
        .into());
    }
    let mut batches = Vec::new();
    let envelope = envelope_bytes(model);
    let mut start = 0usize;
    let mut items = 0usize;
    let mut bytes = envelope;

    for (index, text) in texts.iter().enumerate() {
        let text_bytes = item_bytes(text);

        if items > 0 {
            let candidate_bytes = bytes + 1 + text_bytes;
            if items + 1 > limits.max_items_per_request
                || candidate_bytes > limits.max_request_bytes
            {
                batches.push(EmbeddingBatch { start, end: index });
                start = index;
                items = 0;
                bytes = envelope;
            }
        }

        let separator = usize::from(items > 0);
        bytes += separator + text_bytes;
        items += 1;

        // A batch never exceeds a limit while it is being filled, so an input
        // that overflows one here cannot fit into a request on its own.
        if bytes > limits.max_request_bytes {
            return Err(DomainError::invalid_argument(format!(
                "embedding input {index} does not fit into one provider request body; shorten the \
                 input"
            ))
            .into());
        }
    }

    if items > 0 {
        batches.push(EmbeddingBatch {
            start,
            end: texts.len(),
        });
    }
    Ok(batches)
}
