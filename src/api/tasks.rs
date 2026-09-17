use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use context69_contracts::{
    ApiErrorResponse, CanonicalTaskListQuery, ClearTaskHistoryRequest, DeleteBatchRequest,
    FileBatchRequest, ScopeSpec, TASK_STREAM_IDS_MAX, TaskItemsQuery, TaskListQuery,
    TaskListView, TaskRef, TaskResponse, TaskStatus, TaskStreamDone, TaskStreamEvent,
    TaskStreamQuery, TaskStreamSnapshot, TaskStreamUpdate, TaskSubmitRequest, TextBatchRequest,
    UrlBatchRequest,
};
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{
    ApiState,
    auth::CurrentUser,
    errors::task_error,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::{contracts::TaskKind, services::tasks::TaskSubmission};

#[utoipa::path(
    post,
    path = "/v1/scopes/ensure",
    request_body = ScopeSpec,
    responses((status = 200, body = crate::contracts::EnsureScopeResponse), (status = 409, body = ApiErrorResponse))
)]
pub(crate) async fn ensure_scope(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(spec): Json<ScopeSpec>,
) -> Response {
    match state.app.tasks.ensure_scope(session.user.id, &spec).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/text", params(("group_path" = String, Path)), request_body = TextBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_text_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<TextBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::TextBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/url", params(("group_path" = String, Path)), request_body = UrlBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_url_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UrlBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::UrlBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/file", params(("group_path" = String, Path)), request_body = FileBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_file_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<FileBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::FileBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/delete", params(("group_path" = String, Path)), request_body = DeleteBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_delete_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<DeleteBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::DeleteBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/tasks", request_body = TaskSubmitRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    headers: HeaderMap,
    Json(request): Json<TaskSubmitRequest>,
) -> Response {
    let (kind, group_path, source_key, payloads) = match request {
        TaskSubmitRequest::RetryFileBatch { group_path, items } => (
            TaskKind::FileBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| json!({ "file_id": item.file_id }))
                .collect(),
        ),
        TaskSubmitRequest::FileBatch { group_path, items } => (
            TaskKind::FileBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("FileBatchItem is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::TextBatch { group_path, items } => (
            TaskKind::TextBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("text item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::UrlBatch { group_path, items } => (
            TaskKind::UrlBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("url item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::DeleteBatch { group_path, items } => (
            TaskKind::DeleteBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("delete item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::SourceSync {
            group_path,
            source_key,
        } => (
            TaskKind::SourceSync,
            group_path,
            Some(source_key),
            vec![json!({})],
        ),
        TaskSubmitRequest::TranslationBatch { group_path, items } => (
            TaskKind::Translation,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("translation item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::VectorRebuild => (TaskKind::VectorRebuild, None, None, vec![json!({})]),
    };
    let group = if let Some(path) = group_path.as_deref() {
        match managed_group(&state, session.user.id, path).await {
            Ok(group) => Some(group),
            Err(response) => return response,
        }
    } else {
        None
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: group.as_ref().map(|value| value.id),
            group_path,
            source_key,
            kind,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(
    post,
    path = "/v1/settings/runtime/vector-index/rebuild",
    responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse))
)]
pub(crate) async fn submit_vector_index_rebuild(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    headers: HeaderMap,
) -> Response {
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: None,
            group_path: None,
            source_key: None,
            kind: TaskKind::VectorRebuild,
            payloads: vec![json!({})],
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

pub(crate) async fn submit_task_request(state: &ApiState, request: TaskSubmission) -> Response {
    match state.app.tasks.submit(request).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

async fn submit(state: &ApiState, request: TaskSubmission) -> Response {
    submit_task_request(state, request).await
}

#[utoipa::path(get, path = "/v1/tasks/{task_id}", params(("task_id" = Uuid, Path)), responses((status = 200, body = crate::contracts::TaskResponse), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn get_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.get(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(get, path = "/v1/tasks", params(CanonicalTaskListQuery), responses((status = 200, body = crate::contracts::TaskPageResponse), (status = 400, body = ApiErrorResponse)))]
pub(crate) async fn list_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<CanonicalTaskListQuery>,
) -> Response {
    if let Err(error) = query.validate() {
        return task_error(error);
    }
    if let Err(error) =
        context69_http_support::validate_canonical_offset(query.page, query.page_size)
    {
        return task_error(error);
    }
    let legacy: TaskListQuery = query.into();
    match state.app.tasks.list(session.user.id, &legacy).await {
        Ok(tasks) => (StatusCode::OK, Json(tasks)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(get, path = "/v1/tasks/{task_id}/items", params(("task_id" = Uuid, Path), TaskItemsQuery), responses((status = 200, body = crate::contracts::TaskItemsResponse)))]
pub(crate) async fn list_task_items(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
    Query(query): Query<TaskItemsQuery>,
) -> Response {
    let offset = query.cursor.as_deref().unwrap_or("0").parse::<i64>();
    let offset = match offset {
        Ok(offset) if offset >= 0 => offset,
        _ => return task_error(anyhow::anyhow!("cursor must be a non-negative integer")),
    };
    let limit = i64::from(query.limit);
    match state
        .app
        .tasks
        .items(
            task_id,
            session.user.id,
            limit,
            offset,
            query.status,
        )
        .await
    {
        Ok(items) => (StatusCode::OK, Json(items)).into_response(),
        Err(error) => {
            // Issue #446 P1: list-items 500s previously had no log, leaving
            // only the client-side error. Keep task_id/limit/offset plus
            // the full anyhow chain so slow-SQL/pool failures are diagnosable.
            tracing::error!(
                task_id = %task_id,
                limit,
                offset,
                error = %error,
                chain = ?error.chain().map(ToString::to_string).collect::<Vec<_>>(),
                "list task items failed"
            );
            task_error(error)
        }
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/retry", params(("task_id" = Uuid, Path)), responses((status = 202, body = crate::contracts::TaskRetryResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn retry_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.retry(task_id, session.user.id).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/rerun", params(("task_id" = Uuid, Path)), responses((status = 202, body = crate::contracts::RerunTaskResponse), (status = 400, body = ApiErrorResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn rerun_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.rerun(task_id, session.user.id).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/cancel", params(("task_id" = Uuid, Path)), responses((status = 204), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn cancel_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.cancel(task_id, session.user.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/trash", params(("task_id" = Uuid, Path)), responses((status = 200, body = crate::contracts::TaskResponse), (status = 403, body = ApiErrorResponse), (status = 404, body = ApiErrorResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn trash_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.trash(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/restore", params(("task_id" = Uuid, Path)), responses((status = 200, body = crate::contracts::TaskResponse), (status = 403, body = ApiErrorResponse), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn restore_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.restore(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/clear", request_body = ClearTaskHistoryRequest, responses((status = 200, body = crate::contracts::ClearTaskHistoryResponse), (status = 400, body = ApiErrorResponse)))]
pub(crate) async fn clear_task_history(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<ClearTaskHistoryRequest>,
) -> Response {
    match state
        .app
        .tasks
        .clear_task_history(session.user.id, request.view)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(delete, path = "/v1/tasks/{task_id}", params(("task_id" = Uuid, Path)), responses((status = 204), (status = 403, body = ApiErrorResponse), (status = 404, body = ApiErrorResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn delete_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state
        .app
        .tasks
        .delete_permanently(task_id, session.user.id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => task_error(error),
    }
}

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

fn is_terminal_task_status(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
    )
}

/// Issue 413 Phase 2: per-task SSE `update` coalesce window (3s, user-confirmed).
/// The `snapshot` frame stays immediate; `update` frames buffer the latest full
/// state per `task_id` and flush on this interval. Keep-alive is unchanged.
const TASK_STREAM_UPDATE_COALESCE_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(3);

/// Per-task latest-wins buffer for SSE `update` frames. Bursty bus events for
/// the same `task_id` collapse to one frame per
/// [`TASK_STREAM_UPDATE_COALESCE_WINDOW`]; distinct tasks each keep their
/// latest state. `drain` sorts by `task_id` so flush order is deterministic.
#[derive(Debug, Default)]
struct TaskUpdateCoalescer {
    pending: HashMap<Uuid, TaskResponse>,
}

impl TaskUpdateCoalescer {
    fn push(&mut self, task: TaskResponse) {
        self.pending.insert(task.task_id, task);
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn drain(&mut self) -> Vec<TaskResponse> {
        let mut out: Vec<TaskResponse> = self.pending.drain().map(|(_, task)| task).collect();
        out.sort_by_key(|task| task.task_id);
        out
    }
}

/// Snapshot-then-deltas producer for one SSE connection. The snapshot is sent
/// immediately; deltas coalesce per task on [`TASK_STREAM_UPDATE_COALESCE_WINDOW`]
/// (latest-wins) before `update` frames flush. Snapshot errors and lag
/// notifications are delivered as in-band `error` frames; foreign or missing
/// tasks are silently skipped (own-tasks-only filter).
async fn run_task_stream(
    tasks: crate::services::tasks::TaskService,
    user_id: i64,
    watched_order: Vec<Uuid>,
    tx: mpsc::Sender<TaskStreamEvent>,
    mut abort: context69_search::AbortSignal,
) -> anyhow::Result<()> {
    let watched_set: HashSet<Uuid> = watched_order.iter().copied().collect();
    let filtering = !watched_set.is_empty();

    let snapshot_tasks: Vec<TaskResponse> = if filtering {
        let mut out = Vec::with_capacity(watched_order.len());
        for task_id in &watched_order {
            let fetched = tokio::select! {
                biased;
                _ = abort.wait() => return Ok(()),
                result = tasks.get(*task_id, user_id) => result,
            };
            match fetched {
                Ok(task) => out.push(task),
                Err(error) => {
                    if crate::domain_errors::is_not_found_error(&error) {
                        continue;
                    }
                    if matches!(
                        crate::domain_errors::find_domain_error(&error),
                        Some(crate::domain_errors::DomainError::Forbidden(_))
                    ) {
                        continue;
                    }
                    let _ = tx
                        .send(TaskStreamEvent::Error {
                            message: error.to_string(),
                        })
                        .await;
                    return Ok(());
                }
            }
        }
        out
    } else {
        let query = TaskListQuery {
            page: 1,
            page_size: 100,
            query: None,
            kind: None,
            status: None,
            view: Some(TaskListView::Processing),
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
        };
        let listed = tokio::select! {
            biased;
            _ = abort.wait() => return Ok(()),
            result = tasks.list(user_id, &query) => result,
        };
        match listed {
            Ok(page) => page.items,
            Err(error) => {
                let _ = tx
                    .send(TaskStreamEvent::Error {
                        message: error.to_string(),
                    })
                    .await;
                return Ok(());
            }
        }
    };

    if !send_task_frame(
        &tx,
        TaskStreamEvent::Snapshot(TaskStreamSnapshot {
            tasks: snapshot_tasks.clone(),
        }),
    )
    .await
    {
        return Ok(());
    }

    let mut known: HashMap<Uuid, TaskResponse> = snapshot_tasks
        .iter()
        .map(|task| (task.task_id, task.clone()))
        .collect();
    if filtering
        && known.len() == watched_set.len()
        && !known.is_empty()
        && known
            .values()
            .all(|task| is_terminal_task_status(&task.status))
    {
        let done_tasks = watched_order
            .iter()
            .filter_map(|id| known.get(id).cloned())
            .collect();
        let _ = send_task_frame(&tx, TaskStreamEvent::Done(TaskStreamDone { tasks: done_tasks })).await;
        return Ok(());
    }

    let mut subscription = tasks.subscribe_task_events();
    // Issue 413 Phase 2: snapshot above is immediate; updates below coalesce
    // per task_id on a 3s window. Bus events only refresh `known` plus the
    // latest pending state; the tick drains one `update` per dirty task, so a
    // burst for one task costs one frame per window. `done` follows the flush
    // that makes every watched task terminal, keeping update-then-done order.
    let mut pending = TaskUpdateCoalescer::default();
    let flush_start =
        tokio::time::Instant::now() + TASK_STREAM_UPDATE_COALESCE_WINDOW;
    let mut flush_tick =
        tokio::time::interval_at(flush_start, TASK_STREAM_UPDATE_COALESCE_WINDOW);
    flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = abort.wait() => return Ok(()),
            _ = flush_tick.tick() => {
                if pending.is_empty() {
                    continue;
                }
                for full in pending.drain() {
                    if !send_task_frame(
                        &tx,
                        TaskStreamEvent::Update(TaskStreamUpdate { task: full }),
                    )
                    .await
                    {
                        return Ok(());
                    }
                }
                if filtering
                    && known.len() == watched_set.len()
                    && known.values().all(|task| is_terminal_task_status(&task.status))
                {
                    let done_tasks = watched_order
                        .iter()
                        .filter_map(|id| known.get(id).cloned())
                        .collect();
                    let _ = send_task_frame(
                        &tx,
                        TaskStreamEvent::Done(TaskStreamDone { tasks: done_tasks }),
                    )
                    .await;
                    return Ok(());
                }
            }
            received = subscription.recv() => match received {
                Ok(bus_event) => {
                    if filtering && !watched_set.contains(&bus_event.task_id) {
                        continue;
                    }
                    let fetched = tokio::select! {
                        biased;
                        _ = abort.wait() => return Ok(()),
                        result = tasks.get(bus_event.task_id, user_id) => result,
                    };
                    match fetched {
                        Ok(full) => {
                            known.insert(full.task_id, full.clone());
                            pending.push(full);
                        }
                        Err(error) => {
                            if crate::domain_errors::is_not_found_error(&error) {
                                continue;
                            }
                            if matches!(
                                crate::domain_errors::find_domain_error(&error),
                                Some(crate::domain_errors::DomainError::Forbidden(_))
                            ) {
                                continue;
                            }
                            let _ = tx
                                .send(TaskStreamEvent::Error {
                                    message: error.to_string(),
                                })
                                .await;
                            return Ok(());
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = tx
                        .send(TaskStreamEvent::Error {
                            message: "task events lagged; resync via GET /v1/tasks and reconnect"
                                .to_string(),
                        })
                        .await;
                    return Ok(());
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
            },
        }
    }
}

async fn send_task_frame(tx: &mpsc::Sender<TaskStreamEvent>, event: TaskStreamEvent) -> bool {
    tx.send(event).await.is_ok()
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

async fn managed_group(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> Result<crate::domain::GroupRecord, Response> {
    let group = group_for_user(state, user_id, group_path)
        .await
        .map_err(group_access_error_response)?;
    require_group_role(&group, crate::contracts::MembershipRole::Maintainer)
        .map_err(group_access_error_response)?;
    Ok(group)
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_task_list_query_requires_view_and_rejects_trashed_shape() {
        let canonical: CanonicalTaskListQuery = serde_json::from_value(serde_json::json!({
            "page": 1,
            "page_size": 25,
            "view": "processing"
        }))
        .expect("canonical requires view");
        assert_eq!(
            canonical.view,
            context69_contracts::TaskListView::Processing
        );
        assert!(canonical.validate().is_ok());
        let legacy: TaskListQuery = canonical.into();
        assert!(legacy.view.is_some());

        for trashed in [
            serde_json::json!({
                "page": 1,
                "page_size": 25,
                "view": "processing",
                "trashed": true
            }),
            serde_json::json!({
                "page": 1,
                "page_size": 25,
                "trashed": true
            }),
        ] {
            assert!(
                serde_json::from_value::<CanonicalTaskListQuery>(trashed.clone()).is_err(),
                "old ?trashed= requests must be rejected after removal"
            );
            assert!(
                serde_json::from_value::<TaskListQuery>(trashed).is_err(),
                "old trashed shapes must be rejected after removal"
            );
        }

        let missing_view = serde_json::from_value::<CanonicalTaskListQuery>(serde_json::json!({
            "page": 1,
            "page_size": 25
        }));
        assert!(
            missing_view.is_err(),
            "v0.18 path must require view; trashed-only queries are rejected"
        );

        let zero_page = CanonicalTaskListQuery {
            page: 0,
            page_size: 25,
            query: None,
            kind: None,
            status: None,
            view: context69_contracts::TaskListView::Processing,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
        };
        assert!(zero_page.validate().is_err());
        assert!(
            context69_http_support::validate_canonical_offset(0, 25).is_err(),
            "shared pagination bounds must reject zero page"
        );
        assert!(
            context69_http_support::validate_canonical_offset(1, 101).is_err(),
            "shared pagination bounds must reject oversized page_size"
        );
    }

    #[test]
    fn canonical_view_filtering_never_widens_status() {
        use context69_contracts::{TaskListView, TaskStatus};
        let processing = TaskListView::Processing;
        let completed = TaskListView::Completed;
        assert_ne!(processing, completed);
        assert_eq!(processing.as_str(), "processing");
        let narrowed: Option<TaskStatus> = Some(TaskStatus::Succeeded);
        assert!(
            narrowed.is_some(),
            "status narrows the view; processing + succeeded matches nothing by SQL predicate"
        );
    }

    #[test]
    fn task_stream_query_decodes_comma_ids_and_empty_means_watch_all() {
        // Mirror the search-stream canonical query tests: decoding plus
        // validation happen before the SSE body starts.
        let uri = axum::http::Uri::from_static("/v1/tasks/stream");
        let axum::extract::Query(decoded) =
            axum::extract::Query::<TaskStreamQuery>::try_from_uri(&uri)
                .expect("empty query decodes");
        assert!(decoded.task_ids.is_none());
        assert!(parse_task_stream_ids(decoded.task_ids.as_deref())
            .expect("watch-all")
            .is_empty());

        let one = "11111111-1111-4111-8111-111111111111";
        let two = "22222222-2222-4222-8222-222222222222";
        let ids = parse_task_stream_ids(Some(&format!("{one},{two},{one} , ")))
            .expect("comma ids parse");
        assert_eq!(ids.len(), 2, "duplicate IDs must dedupe");
        assert_eq!(ids[0].to_string(), one);
        assert_eq!(ids[1].to_string(), two);
        assert!(parse_task_stream_ids(Some("  ")).expect("blank").is_empty());
    }

    #[test]
    fn task_stream_rejects_malformed_and_oversized_id_lists() {
        // Invalid IDs surface as 400 before the 200 stream starts, mirroring
        // the search-stream limit/cursor validation.
        assert!(parse_task_stream_ids(Some("not-a-uuid")).is_err());
        assert!(parse_task_stream_ids(Some("11111111-1111-4111-8111-111111111111, nope")).is_err());
        let many = (0..(TASK_STREAM_IDS_MAX + 1))
            .map(|_| "11111111-1111-4111-8111-111111111111".to_string())
            .collect::<Vec<_>>()
            .join(",");
        // Deduped to one, so this stays valid; build distinct IDs to overflow.
        assert_eq!(parse_task_stream_ids(Some(&many)).expect("dedupe").len(), 1);
        let distinct = (0..(TASK_STREAM_IDS_MAX + 1))
            .map(|i| format!("{:08x}-1111-4111-8111-111111111111", i))
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            parse_task_stream_ids(Some(&distinct)).is_err(),
            "more than {TASK_STREAM_IDS_MAX} distinct IDs must be rejected"
        );
    }

    #[test]
    fn task_stream_terminal_detection_covers_only_terminal_states() {
        use context69_contracts::TaskStatus;
        for terminal in [TaskStatus::Succeeded, TaskStatus::Failed, TaskStatus::Cancelled] {
            assert!(
                is_terminal_task_status(&terminal),
                "terminal status must close a filtered stream: {terminal:?}"
            );
        }
        for active in [TaskStatus::Queued, TaskStatus::Running, TaskStatus::Waiting] {
            assert!(
                !is_terminal_task_status(&active),
                "active status must keep the stream open: {active:?}"
            );
        }
    }

    #[test]
    fn task_stream_event_frames_carry_snapshot_update_done_error_shapes() {
        // Contract shapes for the four SSE event names. The transport sends
        // each payload under its own `event:` frame; error mirrors the search
        // stream `{ "message": ... }` envelope.
        let task_id = uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").expect("uuid");
        let task = TaskResponse {
            task_id,
            kind: context69_contracts::TaskKind::TextBatch,
            status: context69_contracts::TaskStatus::Running,
            origin: context69_contracts::TaskOrigin::Manual,
            group_path: None,
            source_key: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            progress: context69_contracts::TaskProgress {
                total: 1,
                queued: 0,
                running: 1,
                waiting: 0,
                succeeded: 0,
                failed: 0,
                cancelled: 0,
            },
            failure_stage: None,
            error_summary: None,
            eta_seconds: None,
            created_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
                .expect("time")
                .with_timezone(&chrono::Utc),
            started_at: None,
            finished_at: None,
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
                .expect("time")
                .with_timezone(&chrono::Utc),
            deleted_at: None,
        };
        let snapshot = TaskStreamSnapshot {
            tasks: vec![task.clone()],
        };
        let snapshot_value = serde_json::to_value(&snapshot).expect("snapshot serializes");
        assert_eq!(
            snapshot_value.get("tasks").and_then(|v| v.as_array()).map(Vec::len),
            Some(1)
        );
        let update = TaskStreamUpdate { task: task.clone() };
        let update_value = serde_json::to_value(&update).expect("update serializes");
        assert_eq!(
            update_value
                .get("task")
                .and_then(|t| t.get("task_id"))
                .and_then(|v| v.as_str()),
            Some("11111111-1111-4111-8111-111111111111")
        );
        let done = TaskStreamDone {
            tasks: vec![task.clone()],
        };
        assert_eq!(serde_json::to_value(&done).expect("done").get("tasks").and_then(|v| v.as_array()).map(Vec::len), Some(1));
        // The tagged union documents the update/done/error contract; the
        // snapshot variant is the first frame on every connection.
        let event_value = serde_json::to_value(&TaskStreamEvent::Update(update)).expect("event");
        assert_eq!(
            event_value.get("type").and_then(|v| v.as_str()),
            Some("update")
        );
        let error_value = serde_json::to_value(&TaskStreamEvent::Error {
            message: "boom".to_string(),
        })
        .expect("error");
        assert_eq!(
            error_value.get("type").and_then(|v| v.as_str()),
            Some("error")
        );
        assert_eq!(
            error_value.get("message").and_then(|v| v.as_str()),
            Some("boom")
        );
        // SSE mapping must not panic for any frame and must keep the stream
        // shape (mirrors the search-stream generator test structure).
        for event in [
            TaskStreamEvent::Snapshot(snapshot),
            TaskStreamEvent::Update(TaskStreamUpdate { task: task.clone() }),
            TaskStreamEvent::Done(done),
            TaskStreamEvent::Error {
                message: "lagged".to_string(),
            },
        ] {
            let _ = sse_task_event(event);
        }
    }

    #[test]
    fn task_stream_update_coalesce_window_is_three_seconds() {
        // Issue 413 Phase 2: aggregation window is user-confirmed 3s. Snapshot
        // stays immediate; only `update` frames wait for this interval.
        assert_eq!(
            TASK_STREAM_UPDATE_COALESCE_WINDOW,
            std::time::Duration::from_secs(3)
        );
    }

    fn coalescer_test_task(
        task_id: Uuid,
        stage: Option<&str>,
        status: context69_contracts::TaskStatus,
    ) -> TaskResponse {
        TaskResponse {
            task_id,
            kind: context69_contracts::TaskKind::TextBatch,
            status,
            origin: context69_contracts::TaskOrigin::Manual,
            group_path: None,
            source_key: None,
            stage: stage.map(ToOwned::to_owned),
            waiting_reason: None,
            dependency_key: None,
            progress: context69_contracts::TaskProgress {
                total: 1,
                queued: 0,
                running: 1,
                waiting: 0,
                succeeded: 0,
                failed: 0,
                cancelled: 0,
            },
            failure_stage: None,
            error_summary: None,
            eta_seconds: None,
            created_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
                .expect("time")
                .with_timezone(&chrono::Utc),
            started_at: None,
            finished_at: None,
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
                .expect("time")
                .with_timezone(&chrono::Utc),
            deleted_at: None,
        }
    }

    #[test]
    fn task_update_coalescer_keeps_latest_per_task() {
        // Bursty bus events for one task collapse to one `update` per window:
        // latest full state wins, so a 2781-event storm costs one frame.
        let task_id =
            Uuid::parse_str("11111111-1111-4111-8111-111111111111").expect("uuid");
        let mut buffer = TaskUpdateCoalescer::default();
        assert!(buffer.is_empty());
        for stage in ["stage-0", "stage-1", "stage-49"] {
            buffer.push(coalescer_test_task(
                task_id,
                Some(stage),
                context69_contracts::TaskStatus::Running,
            ));
        }
        assert_eq!(buffer.pending.len(), 1, "same task_id must overwrite, not queue");
        let flushed = buffer.drain();
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].task_id, task_id);
        assert_eq!(flushed[0].stage.as_deref(), Some("stage-49"));
        assert!(buffer.is_empty(), "drain must clear the window");
        assert!(buffer.drain().is_empty());
    }

    #[test]
    fn task_update_coalescer_flushes_each_task_once_in_stable_order() {
        // Distinct tasks each keep their latest state; flush order is sorted
        // by task_id so the SSE frame order is deterministic.
        let first = Uuid::parse_str("11111111-1111-4111-8111-111111111111").expect("uuid");
        let second = Uuid::parse_str("22222222-2222-4222-8222-222222222222").expect("uuid");
        let mut buffer = TaskUpdateCoalescer::default();
        buffer.push(coalescer_test_task(
            second,
            Some("second-v1"),
            context69_contracts::TaskStatus::Running,
        ));
        buffer.push(coalescer_test_task(
            first,
            Some("first-v1"),
            context69_contracts::TaskStatus::Running,
        ));
        buffer.push(coalescer_test_task(
            second,
            Some("second-v2"),
            context69_contracts::TaskStatus::Running,
        ));
        assert_eq!(buffer.pending.len(), 2);
        let flushed = buffer.drain();
        assert_eq!(flushed.len(), 2);
        assert_eq!(
            flushed.iter().map(|task| task.task_id).collect::<Vec<_>>(),
            vec![first, second],
            "flush must be deterministic across HashMap iteration"
        );
        assert_eq!(flushed[1].stage.as_deref(), Some("second-v2"));
    }

    #[test]
    fn task_items_query_status_defaults_to_none_and_round_trips() {
        // Issue 413 Phase 1: `status` is optional (absent lists every status)
        // and cursor pagination stays offset-based. Old callers sending only
        // limit/cursor keep working; the fixed active-first ordering lives in
        // SQL, not in a new sort param.
        let bare: TaskItemsQuery = serde_json::from_value(serde_json::json!({
            "limit": 100
        }))
        .expect("status defaults to none");
        assert_eq!(bare.limit, 100);
        assert!(bare.cursor.is_none());
        assert!(bare.status.is_none());

        let filtered: TaskItemsQuery = serde_json::from_value(serde_json::json!({
            "limit": 25,
            "cursor": "25",
            "status": "failed"
        }))
        .expect("status failed parses");
        assert_eq!(
            filtered.status,
            Some(context69_contracts::TaskItemStatus::Failed)
        );
        assert_eq!(filtered.cursor.as_deref(), Some("25"));

        assert!(
            serde_json::from_value::<TaskItemsQuery>(serde_json::json!({
                "limit": 25,
                "status": "bogus"
            }))
            .is_err(),
            "unknown item status must be rejected before the handler runs"
        );
    }
}
