//! Validation and encoding of one remote Docling task id.
//!
//! A task id travels into a URL path segment, so it is validated before it is
//! used and percent-encoded so it can never change the request path.

use crate::docling::error::DoclingError;

/// Maximum accepted remote task id length in bytes.
///
/// Task ids are fork-assigned opaque strings (UUID-shaped today). The bound
/// keeps a malicious listing row from turning a cancel/status path into an
/// unbounded request line; legitimate ids are far shorter.
pub(crate) const MAX_TASK_ID_BYTES: usize = 256;

/// Accepts only an opaque, single-segment task id.
///
/// An empty id, an id longer than the bound, and an id carrying whitespace or
/// a control character are all rejected before a request is built.
pub(crate) fn validate_task_id(task_id: &str) -> Result<(), DoclingError> {
    if task_id.is_empty() || task_id.len() > MAX_TASK_ID_BYTES {
        return Err(DoclingError::Validation {
            parameter: "task_id".to_string(),
            reason: "the remote task id must be 1..=256 bytes".to_string(),
        });
    }
    if task_id
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(DoclingError::Validation {
            parameter: "task_id".to_string(),
            reason: "the remote task id must not contain whitespace or control characters"
                .to_string(),
        });
    }
    Ok(())
}

/// Percent-encodes one task id for a URL path segment.
///
/// Only unreserved characters pass through; everything else becomes `%XX`.
/// This keeps a `/`, `?`, or `#` inside an id from changing the request path.
pub(crate) fn encode_task_id_segment(task_id: &str) -> String {
    let mut out = String::with_capacity(task_id.len());
    for byte in task_id.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
