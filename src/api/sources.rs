use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use context69_contracts::TaskKind;
use serde_json::json;

use crate::{
    contracts::{
        ApiErrorResponse, SourceConfigInput, SourceConnectionResponse, SourcePageQuery,
        SourcePageResponse, SourceStatus, TaskRef, UpsertSourceConnectionRequest,
    },
    services::tasks::TaskSubmission,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::source_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
    submit_task_request,
};

#[utoipa::path(
    get,
    path = "/v1/sources",
    params(SourcePageQuery),
    responses(
        (status = 200, description = "Paginated configured sources", body = SourcePageResponse),
        (status = 400, description = "Invalid pagination parameters", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_sources(
    State(state): State<ApiState>,
    Query(query): Query<SourcePageQuery>,
) -> impl IntoResponse {
    if let Err(error) = state.app.sync.reload_sources().await {
        return super::errors::internal_error_response(error);
    }
    match state
        .app
        .sync
        .list_sources_page(query.page, query.page_size, query.query.as_deref())
        .await
    {
        Ok(sources) => (StatusCode::OK, Json(sources)).into_response(),
        Err(error) => super::errors::internal_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/source-connections",
    responses(
        (status = 200, description = "List configured source connections", body = [SourceConnectionResponse]),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_source_connections(State(state): State<ApiState>) -> impl IntoResponse {
    match state.app.sync.list_source_connections().await {
        Ok(connections) => (StatusCode::OK, Json(connections)).into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/source-connections",
    request_body = UpsertSourceConnectionRequest,
    responses(
        (status = 200, description = "Saved source connection", body = SourceConnectionResponse),
        (status = 400, description = "Invalid source connection", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_source_connection(
    State(state): State<ApiState>,
    Json(request): Json<UpsertSourceConnectionRequest>,
) -> impl IntoResponse {
    save_source_connection(state, request).await
}

#[utoipa::path(
    put,
    path = "/v1/source-connections",
    request_body = UpsertSourceConnectionRequest,
    responses(
        (status = 200, description = "Saved source connection", body = SourceConnectionResponse),
        (status = 400, description = "Invalid source connection", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_source_connection(
    State(state): State<ApiState>,
    Json(request): Json<UpsertSourceConnectionRequest>,
) -> impl IntoResponse {
    save_source_connection(state, request).await
}

async fn save_source_connection(
    state: ApiState,
    request: UpsertSourceConnectionRequest,
) -> axum::response::Response {
    match state.app.sync.upsert_source_connection(&request).await {
        Ok(connection) => (StatusCode::OK, Json(connection)).into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/source-connections/{name}",
    params(("name" = String, Path, description = "Source connection name")),
    responses(
        (status = 204, description = "Deleted source connection"),
        (status = 400, description = "Invalid source connection", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_source_connection(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.app.sync.delete_source_connection(&name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/sources",
    request_body = SourceConfigInput,
    responses(
        (status = 201, description = "Created source", body = SourceStatus),
        (status = 400, description = "Invalid source config", body = ApiErrorResponse),
        (status = 409, description = "Source already exists", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_source(
    State(state): State<ApiState>,
    Json(request): Json<SourceConfigInput>,
) -> impl IntoResponse {
    match state.app.sync.create_source(&request).await {
        Ok(source) => (StatusCode::CREATED, Json(source)).into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/sources/{source_key}",
    params(("source_key" = String, Path, description = "Source key")),
    request_body = SourceConfigInput,
    responses(
        (status = 200, description = "Updated source", body = SourceStatus),
        (status = 400, description = "Invalid source config", body = ApiErrorResponse),
        (status = 404, description = "Source not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_source(
    State(state): State<ApiState>,
    Path(source_key): Path<String>,
    Json(request): Json<SourceConfigInput>,
) -> impl IntoResponse {
    match state.app.sync.update_source(&source_key, &request).await {
        Ok(source) => (StatusCode::OK, Json(source)).into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/sources/{source_key}",
    params(("source_key" = String, Path, description = "Source key")),
    responses(
        (status = 204, description = "Deleted source and indexed data"),
        (status = 404, description = "Source not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_source(
    State(state): State<ApiState>,
    Path(source_key): Path<String>,
) -> impl IntoResponse {
    match state.app.sync.delete_source(&source_key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/sources/{source_key}/sync",
    params(("source_key" = String, Path, description = "Source key")),
    responses(
        (status = 202, description = "Source sync task accepted", body = TaskRef),
        (status = 409, description = "Source sync is already running elsewhere", body = ApiErrorResponse),
        (status = 404, description = "Source not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn sync_source(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(source_key): Path<String>,
) -> impl IntoResponse {
    let source = match state.app.sync.list_sources().await {
        Ok(sources) => match sources
            .into_iter()
            .find(|source| source.source_key == source_key)
        {
            Some(source) => source,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiErrorResponse::new(
                        "not_found",
                        format!("unknown source {source_key}"),
                    )),
                )
                    .into_response();
            }
        },
        Err(error) => return super::errors::internal_error_response(error),
    };
    let group = match group_for_user(&state, session.user.id, &source.group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, crate::contracts::MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group.group_path),
            source_key: Some(source_key),
            kind: TaskKind::SourceSync,
            payloads: vec![json!({})],
            idempotency_key: None,
        },
    )
    .await
}
