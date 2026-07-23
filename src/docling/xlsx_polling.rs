use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde_json::Value;
use tokio::time::sleep;

use super::client::ensure_success;
use crate::retry;

pub(super) async fn wait_for_result(
    http: &Client,
    base_url: &str,
    task_id: &str,
    poll_interval: Duration,
    task_timeout: Duration,
) -> Result<Value> {
    let started = Instant::now();
    let deadline = started + task_timeout;
    let mut last_status: Option<String> = None;

    loop {
        let status = poll_status(
            http,
            base_url,
            task_id,
            deadline,
            started,
            last_status.as_deref(),
        )
        .await
        .map_err(|error| anyhow!("failed to poll Docling task {task_id}: {error}"))?;
        last_status = Some(status.clone());

        match status.as_str() {
            "success" => {
                return fetch_result(http, base_url, task_id, deadline, started)
                    .await
                    .map_err(|error| {
                        anyhow!("failed to fetch Docling task result {task_id}: {error}")
                    });
            }
            "failure" | "revoked" => {
                return Err(anyhow!(
                    "docling task {task_id} failed with status {status}"
                ));
            }
            _ => {
                if !sleep_until_deadline(deadline, poll_interval).await {
                    return Err(task_timeout_error(
                        task_id,
                        started,
                        "waiting for the next status poll",
                        last_status.as_deref(),
                    ));
                }
            }
        }
    }
}

async fn poll_status(
    http: &Client,
    base_url: &str,
    task_id: &str,
    deadline: Instant,
    started: Instant,
    last_status: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/status/poll/{task_id}");
    retry::retry_until(
        deadline,
        "docling status poll",
        |_| task_timeout_error(task_id, started, "polling task status", last_status),
        retry::is_retryable,
        || async {
            let response = http
                .get(&url)
                .send()
                .await
                .context("failed to poll docling status")?;
            let response = ensure_success(response, "Docling task status polling").await?;
            let body = response
                .json::<Value>()
                .await
                .context("failed to parse docling status response")?;
            body.get("task_status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .context("docling status response missing task_status")
        },
    )
    .await
    .map_err(|error| {
        anyhow!(
            "docling task {task_id} status polling failed after {:.1}s; last status: {}; {error}",
            started.elapsed().as_secs_f64(),
            last_status.unwrap_or("unknown")
        )
    })
}

async fn fetch_result(
    http: &Client,
    base_url: &str,
    task_id: &str,
    deadline: Instant,
    started: Instant,
) -> Result<Value> {
    let url = format!("{base_url}/result/{task_id}");
    let body = retry::retry_until(
        deadline,
        "docling result fetch",
        |_| task_timeout_error(task_id, started, "fetching task result", Some("success")),
        retry::is_retryable,
        || async {
            let response = http
                .get(&url)
                .send()
                .await
                .context("failed to fetch docling result")?;
            let response = ensure_success(response, "Docling task result fetch").await?;
            response
                .json::<Value>()
                .await
                .context("failed to parse docling result")
        },
    )
    .await
    .map_err(|error| {
        anyhow!(
            "docling task {task_id} result fetch failed after {:.1}s; last status: success; {error}",
            started.elapsed().as_secs_f64()
        )
    })?;

    Ok(body
        .get("document")
        .and_then(|document| document.get("json_content"))
        .cloned()
        .or_else(|| body.get("json_content").cloned())
        .unwrap_or(body))
}

async fn sleep_until_deadline(deadline: Instant, delay: Duration) -> bool {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return false;
    }
    if remaining <= delay {
        sleep(remaining).await;
        return false;
    }
    sleep(delay).await;
    true
}

fn task_timeout_error(
    task_id: &str,
    started: Instant,
    phase: &str,
    last_status: Option<&str>,
) -> anyhow::Error {
    let status = last_status.unwrap_or("unknown");
    anyhow!(
        "docling task {task_id} timed out after {:.1}s while {phase}; last status: {status}",
        started.elapsed().as_secs_f64()
    )
}
