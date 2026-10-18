//! Bounded retry and failure-text policy for the payload-mirror worker.
//!
//! Kept free of I/O so the retry curve, the lease/timeout ordering, and the
//! sanitized failure text can be tested without a database or Qdrant.

use std::time::Duration;

/// First retry delay. Doubles per failed attempt.
pub const RETRY_BASE_SECONDS: u32 = 5;

/// Retry delay ceiling. A job retries indefinitely at this interval.
pub const RETRY_CAP_SECONDS: u32 = 300;

/// Upper bound for one persisted failure reason.
pub const FAILURE_REASON_MAX_CHARS: usize = 512;

/// Retry delay after `attempts` failed attempts. The curve is
/// `5s, 10s, 20s, 40s, 80s, 160s, 300s, 300s, ...`, so a permanent failure
/// stays visible and never discards the pending job.
pub fn retry_delay(attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(6);
    let seconds = RETRY_BASE_SECONDS
        .saturating_mul(1u32 << exponent)
        .min(RETRY_CAP_SECONDS);
    Duration::from_secs(u64::from(seconds))
}

/// One-line, bounded failure text: control characters collapse to spaces and
/// the first 512 characters survive. No payload content is ever read.
pub fn sanitize_failure_reason(error: &anyhow::Error) -> String {
    let mut text = error.to_string();
    for cause in error.chain().skip(1) {
        text.push_str(": ");
        text.push_str(&cause.to_string());
    }
    let cleaned: String = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(FAILURE_REASON_MAX_CHARS).collect()
}
