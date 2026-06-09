use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use uuid::Uuid;

use crate::contracts::{
    CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse,
    LibraryIngestJobResponse, LibraryTreeResponse, LibraryUploadResponse,
    MembershipRole, MoveFileRequest, MoveFolderRequest,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    library_upload::read_library_uploads,
    project_access::{project_access_error_response, project_for_user, require_project_role},
};

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/tree",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    responses(
        (status = 200, description = "Project library tree", body = LibraryTreeResponse),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn get_project_library_tree(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    match state.app.library.list_tree_in_project(&project).await {
        Ok(tree) => (StatusCode::OK, Json(tree)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/folders",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body = CreateFolderRequest,
    responses(
        (status = 201, description = "Created folder", body = crate::contracts::LibraryFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn create_project_library_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    Json(request): Json<CreateFolderRequest>,
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
        .library
        .create_folder_in_project(&project, &request)
        .await
    {
        Ok(folder) => (StatusCode::CREATED, Json(folder)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/texts",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body = CreateTextRequest,
    responses(
        (status = 201, description = "Created text library entry", body = LibraryUploadResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn create_project_library_text(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    Json(request): Json<CreateTextRequest>,
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
        .library
        .create_text_file_in_project(&project, &request)
        .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}/move",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("folder_id" = Uuid, Path, description = "Folder id")
    ),
    request_body = MoveFolderRequest,
    responses(
        (status = 200, description = "Moved folder", body = crate::contracts::LibraryFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or folder not found")
    )
)]
pub(crate) async fn move_project_library_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, folder_id)): Path<(String, String, Uuid)>,
    Json(request): Json<MoveFolderRequest>,
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
        .library
        .move_folder_in_project(&project, folder_id, &request)
        .await
    {
        Ok(folder) => (StatusCode::OK, Json(folder)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("folder_id" = Uuid, Path, description = "Folder id")
    ),
    responses(
        (status = 204, description = "Deleted folder"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or folder not found")
    )
)]
pub(crate) async fn delete_project_library_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, folder_id)): Path<(String, String, Uuid)>,
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
        .library
        .delete_folder_in_project(&project, folder_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/files/upload",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Accepted uploaded files", body = LibraryUploadResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn upload_project_library_files(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    multipart: Multipart,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }
    let uploads = match read_library_uploads(multipart).await {
        Ok(uploads) => uploads,
        Err(response) => return response,
    };
    match state
        .app
        .library
        .upload_files_in_project(&project, uploads)
        .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("file_id" = Uuid, Path, description = "File id")
    ),
    responses(
        (status = 200, description = "Library file details", body = LibraryFileDetailResponse),
        (status = 404, description = "Project or file not found")
    )
)]
pub(crate) async fn get_project_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, file_id)): Path<(String, String, Uuid)>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    match state.app.library.get_file_in_project(&project, file_id).await {
        Ok(file) => (StatusCode::OK, Json(file)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}/move",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("file_id" = Uuid, Path, description = "File id")
    ),
    request_body = MoveFileRequest,
    responses(
        (status = 200, description = "Moved file", body = LibraryFileDetailResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or file not found")
    )
)]
pub(crate) async fn move_project_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, file_id)): Path<(String, String, Uuid)>,
    Json(request): Json<MoveFileRequest>,
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
        .library
        .move_file_in_project(&project, file_id, &request)
        .await
    {
        Ok(file) => (StatusCode::OK, Json(file)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("file_id" = Uuid, Path, description = "File id")
    ),
    responses(
        (status = 204, description = "Deleted file"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or file not found")
    )
)]
pub(crate) async fn delete_project_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, file_id)): Path<(String, String, Uuid)>,
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
        .library
        .delete_file_in_project(&project, file_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/projects/{project_key}/library/jobs/{job_id}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("job_id" = Uuid, Path, description = "Job id")
    ),
    responses(
        (status = 200, description = "Library ingest job", body = LibraryIngestJobResponse),
        (status = 404, description = "Project or job not found")
    )
)]
pub(crate) async fn get_project_library_job(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, job_id)): Path<(String, String, Uuid)>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    match state.app.library.get_job_in_project(&project, job_id).await {
        Ok(job) => (StatusCode::OK, Json(job)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
