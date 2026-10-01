//! Bounds helpers for bounded public Git content acquisition (issue #681
//! phase 3A). Declared payload sizes are gated before any bytes are read, and
//! streamed blob payloads go through an incremental base64 decoder whose
//! aggregate budget covers encoded input plus decoded output.

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::DecodePaddingMode;
use base64::engine::GeneralPurpose;
use base64::engine::GeneralPurposeConfig;

use crate::domain_errors::DomainError;

/// One aggregate cap shared by encoded input and decoded output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ByteBudget {
    pub max_bytes: usize,
}

impl ByteBudget {
    fn charge(&self, spent: &mut usize, bytes: usize) -> Result<()> {
        *spent = spent.saturating_add(bytes);
        if *spent > self.max_bytes {
            return Err(DomainError::payload_too_large("git_aggregate_limit").into());
        }
        Ok(())
    }
}

/// Validate a declared payload size before any bytes are read.
pub(crate) fn check_declared_length(declared: Option<u64>, budget: ByteBudget) -> Result<()> {
    if declared.is_some_and(|length| length > budget.max_bytes as u64) {
        return Err(DomainError::payload_too_large("git_payload_too_large").into());
    }
    Ok(())
}

/// The only tree entry kind acquisition may return.
pub(crate) const TREE_ENTRY_BLOB_TYPE: &str = "blob";

/// Incremental strict base64 decoder for GitHub blob `content` values.
///
/// Chunks may split anywhere. Complete groups without padding decode
/// immediately; everything from the first `=` onward is held back, because
/// padding is only legal at the end of the stream, and the held-back tail is
/// decoded with a canonical-padding engine at [`Self::finish`]. Data after
/// padding therefore fails instead of silently truncating the payload, and
/// ASCII whitespace is stripped before decoding to match the line wrapping
/// GitHub applies to blob content. Trailing bits inside the final symbols
/// are rejected.
///
/// The shared budget is charged once per encoded response byte and once per
/// decoded byte, so unbounded streams fail at the cap regardless of the
/// encoding ratio.
pub(crate) struct BoundedBase64 {
    /// No padding allowed: used for groups that precede the final tail.
    stream_engine: GeneralPurpose,
    /// Canonical padding required: used for the final tail only.
    tail_engine: GeneralPurpose,
    budget: ByteBudget,
    spent: usize,
    /// Whitespace-stripped encoded bytes held back for the tail decode.
    pending: Vec<u8>,
    /// Whether a `=` has been seen; padding only precedes the stream end.
    padding_seen: bool,
}

impl BoundedBase64 {
    pub(crate) fn new(budget: ByteBudget) -> Self {
        Self {
            stream_engine: engine(DecodePaddingMode::RequireNone),
            tail_engine: engine(DecodePaddingMode::RequireCanonical),
            budget,
            spent: 0,
            pending: Vec::new(),
            padding_seen: false,
        }
    }

    /// Feed one encoded chunk and return everything decodable so far.
    ///
    /// A chunk that crosses the aggregate budget fails with
    /// `git_aggregate_limit`; bytes outside the base64 alphabet (ignoring
    /// whitespace) or padding that is not the end of the stream fail with
    /// `git_blob_encoding`.
    pub(crate) fn push_chunk(&mut self, encoded: &[u8]) -> Result<Vec<u8>> {
        self.budget.charge(&mut self.spent, encoded.len())?;
        let compacted: Vec<u8> = encoded
            .iter()
            .copied()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect();
        if self.padding_seen {
            if !compacted.is_empty() {
                return Err(invalid_encoding());
            }
            return Ok(Vec::new());
        }
        self.pending.extend_from_slice(&compacted);
        if let Some(pad_index) = self.pending.iter().position(|&byte| byte == b'=') {
            // The first `=` ends the stream: only the second pad character
            // may follow, anything else is data after padding.
            if self.pending[pad_index + 1..]
                .iter()
                .any(|&byte| byte != b'=')
            {
                return Err(invalid_encoding());
            }
            self.padding_seen = true;
            let decodable = pad_index - pad_index % 4;
            let mut decoded = Vec::new();
            if decodable > 0 {
                decoded = self
                    .stream_engine
                    .decode(&self.pending[..decodable])
                    .map_err(|_| invalid_encoding())?;
                self.budget.charge(&mut self.spent, decoded.len())?;
            }
            self.pending.drain(..decodable);
            return Ok(decoded);
        }
        let decodable = self.pending.len() - self.pending.len() % 4;
        if decodable > 0 {
            let decoded = self
                .stream_engine
                .decode(&self.pending[..decodable])
                .map_err(|_| invalid_encoding())?;
            self.budget.charge(&mut self.spent, decoded.len())?;
            self.pending.drain(..decodable);
            return Ok(decoded);
        }
        if self.pending.len() > self.budget.max_bytes {
            return Err(DomainError::payload_too_large("git_aggregate_limit").into());
        }
        Ok(Vec::new())
    }

    /// Decode the held-back tail, which must carry canonical padding when
    /// present, and apply the final decoded-size charge.
    pub(crate) fn finish(&mut self) -> Result<Vec<u8>> {
        let tail = std::mem::take(&mut self.pending);
        if tail.is_empty() {
            return Ok(Vec::new());
        }
        let decoded = self
            .tail_engine
            .decode(&tail)
            .map_err(|_| invalid_encoding())?;
        self.budget.charge(&mut self.spent, decoded.len())?;
        Ok(decoded)
    }

    /// Budget consumed so far, for tests and diagnostics.
    #[cfg(test)]
    pub(crate) fn spent(&self) -> usize {
        self.spent
    }
}

fn engine(padding_mode: DecodePaddingMode) -> GeneralPurpose {
    GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(padding_mode)
            .with_decode_allow_trailing_bits(false),
    )
}

fn invalid_encoding() -> anyhow::Error {
    anyhow!(DomainError::invalid_argument("git_blob_encoding"))
}

#[cfg(test)]
#[path = "bounds_tests.rs"]
mod tests;
