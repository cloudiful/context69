use std::{
    future::Future,
    time::{Duration, Instant},
};

use anyhow::{Error, Result};
use tokio::time::sleep;
use tracing::warn;

const MAX_TRANSIENT_RETRIES: u32 = 3;
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

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
    E: Fn() -> Error,
{
    let mut retry_count = 0;
    loop {
        if Instant::now() >= deadline {
            return Err(timeout_error());
        }

        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if should_retry(&error) && retry_count < MAX_TRANSIENT_RETRIES => {
                let delay = RETRY_DELAYS[retry_count as usize];
                warn!(
                    operation = operation_name,
                    attempt = retry_count + 1,
                    max_retries = MAX_TRANSIENT_RETRIES,
                    delay_ms = delay.as_millis() as u64,
                    error = %error,
                    "retrying transient Docling request"
                );

                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining <= delay {
                    sleep(remaining).await;
                    return Err(timeout_error());
                }
                sleep(delay).await;
                retry_count += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn is_retryable(error: &Error) -> bool {
    error.chain().any(|cause| {
        let Some(error) = cause.downcast_ref::<reqwest::Error>() else {
            return false;
        };

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
