//! Typed status and control of the vector index rebuild (issue 1212 P5).
//!
//! The status is a durable projection of the rebuild, not a live progress
//! guess: it names the task the controls act on, so a control can never touch
//! a rebuild the caller did not observe. Every control carries the observed
//! task id, and the backend refuses a task that does not own the rebuild.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use context69_contracts::{ApiErrorResponse, RebuildControlRequest, VectorIndexRebuildStatus};

use super::{ApiState, auth::CurrentUser, errors::task_error};
use crate::domain::UserRecord;

#[utoipa::path(
    get,
    path = "/v1/settings/runtime/vector-index/rebuild",
    responses(
        (status = 200, description = "The durable status of the vector index rebuild", body = VectorIndexRebuildStatus),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 500, description = "The durable state could not be read", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_vector_index_rebuild_status(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
) -> Response {
    match state.app.tasks.vector_rebuild_status(&session.user).await {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/settings/runtime/vector-index/rebuild/pause",
    request_body = RebuildControlRequest,
    responses(
        (status = 200, description = "The pause was requested; the current batch may still finish", body = VectorIndexRebuildStatus),
        (status = 400, description = "The observed task is not a vector rebuild", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 409, description = "The observed task does not own a running rebuild", body = ApiErrorResponse)
    )
)]
pub(crate) async fn pause_vector_index_rebuild(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<RebuildControlRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_pause_vector_rebuild(&session.user, request.task_id)
        .await
    {
        Ok(()) => rebuild_status_response(&state, &session.user).await,
        Err(error) => task_error(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/settings/runtime/vector-index/rebuild/resume",
    request_body = RebuildControlRequest,
    responses(
        (status = 200, description = "The paused rebuild continues after its last written page", body = VectorIndexRebuildStatus),
        (status = 400, description = "The observed task is not a vector rebuild", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 409, description = "The observed task is not paused on a running rebuild", body = ApiErrorResponse)
    )
)]
pub(crate) async fn resume_vector_index_rebuild(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<RebuildControlRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_resume_vector_rebuild(&session.user, request.task_id)
        .await
    {
        Ok(()) => rebuild_status_response(&state, &session.user).await,
        Err(error) => task_error(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/settings/runtime/vector-index/rebuild/cancel",
    request_body = RebuildControlRequest,
    responses(
        (status = 200, description = "The rebuild attempt was cancelled; the existing collections are kept", body = VectorIndexRebuildStatus),
        (status = 400, description = "The observed task is not a vector rebuild", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 409, description = "The observed task is already terminal", body = ApiErrorResponse)
    )
)]
pub(crate) async fn cancel_vector_index_rebuild(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<RebuildControlRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_cancel_vector_rebuild(&session.user, request.task_id)
        .await
    {
        Ok(()) => rebuild_status_response(&state, &session.user).await,
        Err(error) => task_error(error),
    }
}

/// The status the control answered with.
///
/// The control and the status are one round trip: the answer describes the
/// state the control actually produced, not the state before it.
async fn rebuild_status_response(state: &ApiState, actor: &UserRecord) -> Response {
    match state.app.tasks.vector_rebuild_status(actor).await {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(error) => task_error(error),
    }
}
