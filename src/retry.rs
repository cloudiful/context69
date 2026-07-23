use std::{
    error::Error as StdError,
    future::Future,
    time::{Duration, Instant},
};

use anyhow::{Error, Result};
use tokio::time::sleep;
use tracing::warn;

pub(crate) const MAX_TRANSIENT_RETRIES: u32 = 3;
pub(crate) const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

#[derive(Debug)]
pub(crate) struct RetryableError {
    source: Error,
}

impl std::fmt::Display for RetryableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl StdError for RetryableError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(crate) fn mark_retryable(error: Error) -> Error {
    Error::new(RetryableError { source: error })
}

pub(crate) async fn retry_until<T, F, Fut, R, E>(
    deadline: Instant,
    operation_name: &str,
    timeout_error: E,
    should_retry: R,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    R: Fn(&Error) -> bool,
    E: Fn(Option<Error>) -> Error,
{
    let started = Instant::now();
    let mut retry_count = 0;
    let mut last_retry_error = None;
    loop {
        if Instant::now() >= deadline {
            return Err(timeout_error(last_retry_error.take()));
        }

        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if should_retry(&error) && retry_count < MAX_TRANSIENT_RETRIES => {
                let delay = RETRY_DELAYS[retry_count as usize];
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining <= delay {
                    warn!(
                        operation = operation_name,
                        attempt = retry_count + 1,
                        max_retries = MAX_TRANSIENT_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        remaining_ms = remaining.as_millis() as u64,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        error = %error,
                        "retry budget exhausted before transient retry"
                    );
                    return Err(timeout_error(Some(error)));
                }

                warn!(
                    operation = operation_name,
                    attempt = retry_count + 1,
                    max_retries = MAX_TRANSIENT_RETRIES,
                    delay_ms = delay.as_millis() as u64,
                    remaining_ms = remaining.as_millis() as u64,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    error = %error,
                    "retrying transient request"
                );

                last_retry_error = Some(error);
                sleep(delay).await;
                retry_count += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn is_retryable(error: &Error) -> bool {
    error.chain().any(|cause| {
        if cause.downcast_ref::<RetryableError>().is_some() {
            return true;
        }

        let Some(error) = cause.downcast_ref::<reqwest::Error>() else {
            return false;
        };

        if error.is_builder() {
            return false;
        }
        if let Some(status) = error.status() {
            return status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
        }

        error.is_timeout()
            || error.is_connect()
            || error.is_request()
            || error.is_body()
            || error.is_decode()
    })
}
