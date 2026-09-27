//! Task SSE stream handler: the `GET /v1/tasks/stream` endpoint, its query
//! parsing, and the SSE frame mapping. The producer state machine that feeds
//! the frames lives in the adjacent `producer` module.

use std::{collections::HashSet, convert::Infallible};

use axum::{
    extract::{Query, State},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use context69_contracts::{
    ApiErrorResponse, TASK_STREAM_IDS_MAX, TaskStreamEvent, TaskStreamQuery,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{ApiState, CurrentUser, task_error};

mod producer;

use producer::run_task_stream;

/// Subscribe to the caller's own task states over Server-Sent Events.
///
/// Best-effort PG NOTIFY fan-out (issue 405 Task E1): the handler sends one
/// immediate `snapshot` frame with full states, then `update` frames coalesced
/// per task every 3s (issue 413 Phase 2, latest-wins), then `done` when every
/// explicitly watched `task_ids` entry is terminal. `Last-Event-ID` is not
/// replayed; clients re-sync with `GET /v1/tasks` (or `GET /v1/tasks/{task_id}`)
/// on reconnect (E3). Disconnects drop the `AbortOnDrop` in the unfold state,
/// which aborts the producer and unsubscribes the broadcast receiver.
#[utoipa::path(
    get,
    path = "/v1/tasks/stream",
    params(TaskStreamQuery),
    responses(
        (status = 200, description = "Server-Sent Events stream (text/event-stream). Frames: `snapshot` (immediate full states at subscribe time), `update` (one watched task's current full state, coalesced per task every 3s, latest-wins), `done` (all explicitly watched tasks terminal; watch-all streams never send `done`), or `error` (message; client must resync via GET /v1/tasks and reconnect)."),
        (status = 400, body = ApiErrorResponse, description = "Invalid task_ids query (malformed UUID or more than 100 IDs). The SSE body never starts; the 400 is returned directly.")
    )
)]
pub(crate) async fn stream_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<TaskStreamQuery>,
) -> Response {
    let watched = match parse_task_stream_ids(query.task_ids.as_deref()) {
        Ok(ids) => ids,
        Err(error) => return task_error(error),
    };
    let (tx, rx) = mpsc::channel::<TaskStreamEvent>(32);
    let tasks = state.app.tasks.clone();
    let user_id = session.user.id;
    // Mirror the search-stream pattern: the producer selects on `abort`
    // next to its DB/broadcast awaits, and the `AbortOnDrop` lives in the
    // unfold state so a client disconnect aborts the work and drops the
    // broadcast subscription instead of running to completion.
    let (signal, on_drop) = context69_search::abort_pair();
    tokio::spawn(async move {
        let _ = run_task_stream(tasks, user_id, watched, tx, signal).await;
    });
    let stream = futures::stream::unfold((rx, on_drop), |(mut rx, abort)| async move {
        rx.recv()
            .await
            .map(|event| (Ok::<_, Infallible>(sse_task_event(event)), (rx, abort)))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Parse `GET /v1/tasks/stream?task_ids=` (comma-separated UUIDs).
/// `None`/blank means watch-all (own tasks only). Rejects malformed UUIDs
/// and more than [`TASK_STREAM_IDS_MAX`] IDs as typed invalid-argument so
/// the handler returns 400 before the SSE body starts.
fn parse_task_stream_ids(raw: Option<&str>) -> anyhow::Result<Vec<Uuid>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let id: Uuid = part.parse().map_err(|_| {
            crate::domain_errors::DomainError::invalid_argument(format!(
                "invalid task_ids entry (must be UUID): {part}"
            ))
        })?;
        if seen.insert(id) {
            out.push(id);
        }
    }
    if out.len() > TASK_STREAM_IDS_MAX {
        return Err(crate::domain_errors::DomainError::invalid_argument(format!(
            "task_ids must contain at most {TASK_STREAM_IDS_MAX} IDs"
        ))
        .into());
    }
    Ok(out)
}

fn sse_task_event(event: TaskStreamEvent) -> Event {
    match event {
        TaskStreamEvent::Snapshot(snapshot) => named_task_json("snapshot", &snapshot),
        TaskStreamEvent::Update(update) => named_task_json("update", &update),
        TaskStreamEvent::Done(done) => named_task_json("done", &done),
        TaskStreamEvent::Error { message } => {
            named_task_json("error", &serde_json::json!({ "message": message }))
        }
    }
}

fn named_task_json<T: serde::Serialize>(name: &str, value: &T) -> Event {
    let payload = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    Event::default().event(name).data(payload)
}

#[cfg(test)]
mod tests;
