use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    contracts::{MembershipRole, SourceConfigInput, SourceStatus, SyncOutcome},
    services::scheduler::{ManualRunResult, run_manual_sync_guarded},
    source_store::SourceScope,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::source_management_error_response,
    project_access::{project_access_error_response, project_for_user, require_project_role},
};

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/projects/{project_key}/sources",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    responses(
        (status = 200, description = "List project sources", body = [SourceStatus]),
        (status = 404, description = "Project not found"),
        (status = 401, description = "Missing or invalid bearer token")
    )
)]
pub(crate) async fn list_project_sources(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = state.app.sync.reload_sources().await {
        return super::errors::internal_error_response(error);
    }
    match state.app.sync.list_sources_for_project(project.id).await {
        Ok(sources) => (StatusCode::OK, Json(sources)).into_response(),
        Err(error) => super::errors::internal_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/sources",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body = SourceConfigInput,
    responses(
        (status = 201, description = "Created project source", body = SourceStatus),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn create_project_source(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    Json(request): Json<SourceConfigInput>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }

    let scope = SourceScope {
        group_id: project.group_id,
        group_key: project.group_key.clone(),
        project_id: project.id,
        project_key: project.project_key.clone(),
        visibility: project.visibility,
    };
    match state.app.sync.create_source_in_scope(&scope, &request).await {
        Ok(source) => (StatusCode::CREATED, Json(source)).into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("source_key" = String, Path, description = "Source key")
    ),
    request_body = SourceConfigInput,
    responses(
        (status = 200, description = "Updated project source", body = SourceStatus),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or source not found")
    )
)]
pub(crate) async fn update_project_source(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, source_key)): Path<(String, String, String)>,
    Json(request): Json<SourceConfigInput>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }
    match state
        .app
        .sync
        .update_source_in_project(project.id, &source_key, &request)
        .await
    {
        Ok(source) => (StatusCode::OK, Json(source)).into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("source_key" = String, Path, description = "Source key")
    ),
    responses(
        (status = 204, description = "Deleted project source"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or source not found")
    )
)]
pub(crate) async fn delete_project_source(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, source_key)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }
    match state
        .app
        .sync
        .delete_source_in_project(project.id, &source_key)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => source_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}/sync",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("source_key" = String, Path, description = "Source key")
    ),
    responses(
        (status = 202, description = "Triggered project source sync", body = SyncOutcome),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or source not found")
    )
)]
pub(crate) async fn sync_project_source(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, source_key)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }
    match state
        .app
        .sync
        .get_source_for_project(project.id, &source_key)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return source_management_error_response(anyhow::anyhow!(
                "unknown source {source_key}"
            ));
        }
        Err(error) => return source_management_error_response(error),
    }

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
        Ok(ManualRunResult::Contended) => source_management_error_response(anyhow::anyhow!(
            "source {source_key} sync is already running"
        )),
        Err(error) => source_management_error_response(error),
    }
}
