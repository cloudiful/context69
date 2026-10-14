use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::TaskService;
use crate::services::library::docling_remote_ops::{
    DoclingRemoteCancelObservation as Cancel, DoclingRemoteStatusObservation as Status,
    fetch_remote_status, post_remote_cancel,
};

const CLAIM_LIMIT: i64 = 16;
const PASS_LIMIT: Duration = Duration::from_secs(30);

fn backoff(attempt: i32) -> chrono::Duration {
    chrono::Duration::seconds(
        30_i64
            .saturating_mul(1_i64 << attempt.saturating_sub(1).clamp(0, 5))
            .min(900),
    )
}

struct Settlement {
    status: &'static str,
    outcome: Option<String>,
    error: Option<&'static str>,
}

impl Settlement {
    fn new(status: &'static str, outcome: Option<String>, error: Option<&'static str>) -> Self {
        Self {
            status,
            outcome,
            error,
        }
    }
}

fn cancel_result(observation: Cancel) -> Settlement {
    match observation {
        Cancel::Cancelled { observed_status } => {
            Settlement::new("done", Some(observed_status), None)
        }
        Cancel::AwaitingTerminal | Cancel::NotFound => {
            Settlement::new("awaiting_terminal", None, None)
        }
        Cancel::Unsupported => Settlement::new(
            "unsupported",
            None,
            Some("Remote cancellation requires unsupported authentication"),
        ),
        Cancel::Retryable | Cancel::Failed => Settlement::new(
            "pending",
            None,
            Some("Remote cancellation was not confirmed"),
        ),
    }
}

fn status_result(observation: Status) -> Settlement {
    match observation {
        Status::Observed {
            task_status,
            is_terminal: true,
            ..
        } => Settlement::new("done", Some(task_status), None),
        Status::Observed { .. } => Settlement::new("awaiting_terminal", None, None),
        Status::NotFound => Settlement::new(
            "done",
            Some("not_found".into()),
            Some("Remote task not found; termination not confirmed"),
        ),
        Status::Unsupported => Settlement::new(
            "unsupported",
            None,
            Some("Remote observation requires unsupported authentication"),
        ),
        Status::Retryable | Status::Failed => Settlement::new(
            "awaiting_terminal",
            None,
            Some("Remote terminal state was not confirmed"),
        ),
    }
}

async fn observe(
    deadline: DateTime<Utc>,
    endpoint: Option<&str>,
    status: &str,
    remote_id: &str,
) -> Settlement {
    if deadline <= Utc::now() {
        return Settlement::new(
            "failed",
            None,
            Some("Cleanup deadline exceeded; remote termination unconfirmed"),
        );
    }
    let Some(endpoint) = endpoint else {
        return Settlement::new(
            "unsupported",
            None,
            Some("Submit endpoint unavailable; remote termination unconfirmed"),
        );
    };
    if status == "awaiting_terminal" {
        return status_result(
            fetch_remote_status(endpoint, remote_id)
                .await
                .unwrap_or(Status::Retryable),
        );
    }
    cancel_result(
        post_remote_cancel(endpoint, remote_id)
            .await
            .unwrap_or(Cancel::Retryable),
    )
}

async fn process_pairs<I, F>(work: I) -> Result<()>
where
    I: IntoIterator<Item = F>,
    F: std::future::Future<Output = Result<()>>,
{
    let mut work = work.into_iter().take(CLAIM_LIMIT as usize);
    while let Some(first) = work.next() {
        if let Some(second) = work.next() {
            let (a, b) = tokio::join!(first, second);
            a?;
            b?;
        } else {
            first.await?;
        }
    }
    Ok(())
}

pub(super) async fn run_once(service: &TaskService, shutdown: &CancellationToken) -> Result<()> {
    let pass = async {
        let token = Uuid::new_v4();
        let rows = service
            .db()
            .claim_docling_cleanup(CLAIM_LIMIT, token)
            .await?;
        let work = rows.into_iter().map(|row| async move {
            // An adopted same-identity mapping wins over a stale cleanup request.
            if !service
                .db()
                .docling_cleanup_claim_is_detached(row.id, token)
                .await?
            {
                return Ok::<(), anyhow::Error>(());
            }
            let result = observe(
                row.cleanup_deadline_at,
                row.endpoint_base_url.as_deref(),
                &row.status,
                &row.remote_task_id,
            )
            .await;
            let next = (Utc::now() + backoff(row.attempt_count)).min(row.cleanup_deadline_at);
            service
                .db()
                .settle_docling_cleanup(
                    row.id,
                    result.status,
                    result.outcome.as_deref(),
                    result.error,
                    next,
                    token,
                )
                .await?;
            Ok(())
        });
        process_pairs(work).await
    };
    tokio::select! {
        _ = shutdown.cancelled() => Ok(()),
        result = tokio::time::timeout(PASS_LIMIT, pass) => match result {
            Ok(result) => result,
            // Unsettled claims expire after 60s; shutdown never cancels adopted jobs.
            Err(_) => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pass_runs_at_most_sixteen_operations_two_at_a_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let work = (0..100).map(|_| async {
            let concurrent = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(concurrent, Ordering::SeqCst);
            tokio::task::yield_now().await;
            active.fetch_sub(1, Ordering::SeqCst);
            finished.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        process_pairs(work).await.unwrap();
        assert_eq!(finished.load(Ordering::SeqCst), 16);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn legacy_and_deadline_never_request_remote() {
        let legacy = observe(
            Utc::now() + chrono::Duration::hours(1),
            None,
            "pending",
            "x",
        )
        .await;
        assert_eq!(legacy.status, "unsupported");
        let expired = observe(
            Utc::now() - chrono::Duration::seconds(1),
            Some("http://127.0.0.1:1"),
            "pending",
            "x",
        )
        .await;
        assert_eq!(expired.status, "failed");
        assert!(expired.error.unwrap().contains("unconfirmed"));
        assert_eq!(CLAIM_LIMIT, 16);
        assert_eq!(PASS_LIMIT, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn started_job_is_observed_without_result_fetch() {
        use axum::{
            Json, Router,
            routing::{get, post},
        };
        let router = Router::new()
            .route("/v1/tasks/x/cancel", post(|| async { axum::http::StatusCode::CONFLICT }))
            .route("/v1/status/poll/x", get(|query: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                assert_eq!(query.get("wait").map(String::as_str), Some("0"));
                Json(serde_json::json!({"task_id": "x", "task_type": "convert", "task_status": "started"}))
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let deadline = Utc::now() + chrono::Duration::hours(1);
        assert_eq!(
            observe(deadline, Some(&endpoint), "pending", "x")
                .await
                .status,
            "awaiting_terminal"
        );
        assert_eq!(
            observe(deadline, Some(&endpoint), "awaiting_terminal", "x")
                .await
                .status,
            "awaiting_terminal"
        );
        server.abort();
    }

    #[test]
    fn backoff_and_observations_are_conservative() {
        assert_eq!(backoff(1).num_seconds(), 30);
        assert_eq!(backoff(2).num_seconds(), 60);
        assert_eq!(backoff(i32::MAX).num_seconds(), 900);
        assert_eq!(cancel_result(Cancel::NotFound).status, "awaiting_terminal");
        assert_eq!(cancel_result(Cancel::Unsupported).status, "unsupported");
        assert_eq!(cancel_result(Cancel::Failed).status, "pending");
        let actual = cancel_result(Cancel::Cancelled {
            observed_status: "success".into(),
        });
        assert_eq!(actual.outcome.as_deref(), Some("success"));
        let gone = status_result(Status::NotFound);
        assert_eq!(gone.outcome.as_deref(), Some("not_found"));
        assert!(gone.error.unwrap().contains("not confirmed"));
        assert_eq!(
            status_result(Status::Observed {
                task_id: "x".into(),
                task_status: "future".into(),
                is_terminal: false
            })
            .status,
            "awaiting_terminal"
        );
    }
}
