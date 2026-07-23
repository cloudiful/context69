use std::time::{Duration, Instant};

use anyhow::Error;

use super::errors;

pub(super) fn retry_deadline(started: Instant, timeout: Duration) -> Instant {
    started + timeout
}

pub(super) fn finalize_error(
    error: Error,
    attempts: u32,
    started: Instant,
    timeout: Duration,
) -> Error {
    errors::finalize_embedding_error(
        error,
        attempts,
        started.elapsed().as_millis() as u64,
        timeout.as_millis() as u64,
    )
}
