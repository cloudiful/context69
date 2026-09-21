//! Task event bus (issue 405 Task E1).
//!
//! PostgreSQL `LISTEN task_events` in a resident task fans out into a
//! process-local `tokio::broadcast` channel. Every replica LISTENs, so
//! increments committed on any replica (Rust or pure-SQL writers such as
//! cancel/trash/restore/clear, `maintain_claim_state`)
//! are visible to every replica. Best-effort semantics: subscribers must
//! full-sync once on subscribe, then apply incremental events.
//!
//! No HTTP/SSE route lives here; that is Task E2.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use uuid::Uuid;

/// PostgreSQL notification channel. Must match the migration triggers.
pub const TASK_EVENTS_CHANNEL: &str = "task_events";

/// Broadcast capacity. Large enough for bursty recompute/cascade notifies;
/// lagging receivers observe `Lagged` and must full-sync again.
pub const TASK_EVENT_BUS_CAPACITY: usize = 1024;

/// Minimal task event. Payload on the wire is limited to exactly these four
/// fields (see migration `20260916000000_task_events_notify.sql`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEvent {
    pub task_id: Uuid,
    pub item_id: Option<Uuid>,
    pub status: String,
    pub updated_at: DateTime<Utc>,
}

impl TaskEvent {
    /// Parse a `pg_notify` payload. Rejects missing fields and empty status
    /// so a malformed trigger payload never fans out silently.
    pub fn parse_payload(payload: &str) -> anyhow::Result<Self> {
        let event: Self = serde_json::from_str(payload)?;
        if event.status.trim().is_empty() {
            anyhow::bail!("task event status must not be empty");
        }
        Ok(event)
    }

    /// Raw JSON keys carried on the wire. Used by tests to assert the
    /// 4-field payload ceiling.
    pub fn payload_keys(payload: &str) -> anyhow::Result<Vec<String>> {
        let value: serde_json::Value = serde_json::from_str(payload)?;
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("task event payload must be a JSON object"))?;
        let mut keys: Vec<String> = object.keys().cloned().collect();
        keys.sort();
        Ok(keys)
    }
}

/// Create a broadcast channel for task events without starting a listener.
/// Tests and the service constructor use this; production also spawns the
/// resident listener via [`spawn_task_event_listener`].
pub fn new_task_event_channel() -> (
    broadcast::Sender<TaskEvent>,
    broadcast::Receiver<TaskEvent>,
) {
    broadcast::channel(TASK_EVENT_BUS_CAPACITY)
}

/// Spawn the resident `PgListener -> broadcast` hub. The task reconnects
/// transparently: `PgListener::recv` re-establishes the connection and the
/// outer loop re-issues `LISTEN` after any terminal error, so a dropped
/// connection recovers without process restart.
pub fn spawn_task_event_listener(pool: sqlx::PgPool) -> broadcast::Sender<TaskEvent> {
    let (sender, _receiver) = broadcast::channel(TASK_EVENT_BUS_CAPACITY);
    let task_sender = sender.clone();
    tokio::spawn(async move {
        run_task_event_listener(pool, task_sender).await;
    });
    sender
}

/// Spawn the hub with shutdown support for tests. Exits when `shutdown`
/// cancels; otherwise behaves like [`spawn_task_event_listener`].
pub fn spawn_task_event_listener_with_shutdown(
    pool: sqlx::PgPool,
    shutdown: tokio_util::sync::CancellationToken,
) -> broadcast::Sender<TaskEvent> {
    let (sender, _receiver) = broadcast::channel(TASK_EVENT_BUS_CAPACITY);
    let task_sender = sender.clone();
    tokio::spawn(async move {
        tokio::select! {
            () = run_task_event_listener(pool, task_sender) => {},
            () = shutdown.cancelled() => {},
        }
    });
    sender
}

/// Spawn the hub into an existing broadcast sender. The service bus uses
/// this so subscribers on `TaskService::subscribe_task_events` observe
/// PG NOTIFY directly without a second forwarding hop.
pub fn spawn_task_event_listener_into(
    pool: sqlx::PgPool,
    sender: broadcast::Sender<TaskEvent>,
) {
    tokio::spawn(async move {
        run_task_event_listener(pool, sender).await;
    });
}

/// Shutdown-aware variant of [`spawn_task_event_listener_into`].
pub fn spawn_task_event_listener_into_with_shutdown(
    pool: sqlx::PgPool,
    sender: broadcast::Sender<TaskEvent>,
    shutdown: tokio_util::sync::CancellationToken,
) {
    tokio::spawn(async move {
        tokio::select! {
            () = run_task_event_listener(pool, sender) => {},
            () = shutdown.cancelled() => {},
        }
    });
}

async fn run_task_event_listener(pool: sqlx::PgPool, sender: broadcast::Sender<TaskEvent>) {
    loop {
        match PgListener::connect_with(&pool).await {
            Ok(mut listener) => {
                if let Err(error) = listener.listen(TASK_EVENTS_CHANNEL).await {
                    tracing::warn!(%error, "task event listener LISTEN failed; retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
                tracing::info!("task event listener subscribed to task_events");
                loop {
                    match listener.recv().await {
                        Ok(notification) => {
                            match TaskEvent::parse_payload(notification.payload()) {
                                Ok(event) => {
                                    let _ = sender.send(event);
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "skipping malformed task event payload");
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "task event listener recv failed; reconnecting");
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "task event listener connect failed; retrying");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskEvent, TASK_EVENT_BUS_CAPACITY, TASK_EVENTS_CHANNEL};

    #[test]
    fn channel_name_is_stable() {
        assert_eq!(TASK_EVENTS_CHANNEL, "task_events");
        const { assert!(TASK_EVENT_BUS_CAPACITY >= 16) };
    }

    #[test]
    fn task_event_payload_round_trip_is_four_fields() {
        let payload = serde_json::json!({
            "task_id": "11111111-1111-4111-8111-111111111111",
            "item_id": null,
            "status": "running",
            "updated_at": "2026-09-16T00:00:00Z",
        })
        .to_string();
        let event = TaskEvent::parse_payload(&payload).expect("parse");
        assert_eq!(event.status, "running");
        assert_eq!(event.item_id, None);
        let mut keys = TaskEvent::payload_keys(&payload).expect("keys");
        keys.sort();
        assert_eq!(keys, ["item_id", "status", "task_id", "updated_at"]);
    }

    #[test]
    fn task_event_rejects_empty_status() {
        let payload = serde_json::json!({
            "task_id": "11111111-1111-4111-8111-111111111111",
            "item_id": null,
            "status": "  ",
            "updated_at": "2026-09-16T00:00:00Z",
        })
        .to_string();
        assert!(TaskEvent::parse_payload(&payload).is_err());
    }

    #[test]
    fn task_event_rejects_malformed_payload() {
        assert!(TaskEvent::parse_payload("not-json").is_err());
        assert!(
            TaskEvent::parse_payload(r#"{"task_id":"x"}"#).is_err(),
            "missing fields must fail"
        );
    }
}
