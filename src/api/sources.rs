use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    contracts::{
        ApiErrorResponse, SourceConfigInput, SourceConnectionResponse, SourceStatus, SyncOutcome,
        UpsertSourceConnectionRequest,
    },
    services::scheduler::{ManualRunResult, run_manual_sync_guarded},
};

use super::{ApiState, errors::source_management_error_response};

#[utoipa::path(
    get,
    path = "/v1/sources",
    responses(
        (status = 200, description = "List configured sources", body = [SourceStatus]),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_sources(State(state): State<ApiState>) -> impl IntoResponse {
    if let Err(error) = state.app.sync.reload_sources().await {
        return super::errors::internal_error_response(error);
    }
    match state.app.sync.list_sources().await {
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
        (status = 202, description = "Triggered source sync", body = SyncOutcome),
        (status = 409, description = "Source sync is already running elsewhere", body = ApiErrorResponse),
        (status = 404, description = "Source not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn sync_source(
    State(state): State<ApiState>,
    Path(source_key): Path<String>,
) -> impl IntoResponse {
    match run_manual_sync_guarded(state.app.clone(), format!("source:{source_key}"), || {
        let app = state.app.clone();
        let source_key = source_key.clone();
        async move { app.sync.sync_source(&source_key, "api").await }
    })
    .await
    {
        Ok(ManualRunResult::Completed(outcome)) => {
            (StatusCode::ACCEPTED, Json(outcome)).into_response()
        }
        Ok(ManualRunResult::Contended) => (
            StatusCode::CONFLICT,
            Json(ApiErrorResponse {
                error: format!("source {source_key} sync is already running"),
            }),
        )
            .into_response(),
        Err(error) if error.to_string().contains("unknown source") => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
        Err(error) => super::errors::internal_error_response(error),
    }
}
