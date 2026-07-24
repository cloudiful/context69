use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use uuid::Uuid;

use crate::contracts::{
    CreateFolderRequest, CreateTextRequest, ImportLibraryFileFromUrlRequest,
    LibraryFileDetailResponse, LibraryFileJobPageQuery, LibraryFileJobPageResponse,
    LibraryIngestJobResponse, LibraryResourcePageQuery, LibraryResourcePageResponse,
    LibraryTreeResponse, LibraryUploadResponse, LibraryUrlImportJobResponse, MembershipRole,
    MoveFileRequest, MoveFolderRequest, PrepareLibraryUploadRequest, PrepareLibraryUploadResponse,
    UpsertLibraryTextRequest,
};

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/import-url",
    params(("group_path" = String, Path)),
    request_body = ImportLibraryFileFromUrlRequest,
    responses((status = 202, body = LibraryUrlImportJobResponse), (status = 403), (status = 404))
)]
pub(crate) async fn import_group_library_file_url(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<ImportLibraryFileFromUrlRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .import_url_in_project(&group, &request)
        .await
    {
        Ok(job) => (StatusCode::ACCEPTED, Json(job)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/library/url-import-jobs/{job_id}",
    params(("group_path" = String, Path), ("job_id" = Uuid, Path)),
    responses((status = 200, body = LibraryUrlImportJobResponse), (status = 404))
)]
pub(crate) async fn get_group_library_url_import_job(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, job_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .library
        .get_url_import_job_in_project(group.id, job_id)
        .await
    {
        Ok(job) => (StatusCode::OK, Json(job)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/url-import-jobs/{job_id}/retry",
    params(("group_path" = String, Path), ("job_id" = Uuid, Path)),
    responses((status = 202, body = LibraryUrlImportJobResponse), (status = 404), (status = 409))
)]
pub(crate) async fn retry_group_library_url_import_job(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, job_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .retry_url_import_job_in_project(group.id, job_id)
        .await
    {
        Ok(job) => (StatusCode::ACCEPTED, Json(job)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
    library_upload::read_library_uploads,
};

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/library/tree",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    responses(
        (status = 200, description = "Group library tree", body = LibraryTreeResponse),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn get_group_library_tree(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = state
        .app
        .source_folders
        .migrate_project_sources_in_project(&group)
        .await
    {
        return library_management_error_response(error);
    }
    match state.app.library.list_tree_in_project(&group).await {
        Ok(tree) => (StatusCode::OK, Json(tree)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/library/resources",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        LibraryResourcePageQuery
    ),
    responses(
        (status = 200, description = "Paginated resources in a group library folder", body = LibraryResourcePageResponse),
        (status = 400, description = "Invalid pagination parameters"),
        (status = 404, description = "Group or folder not found")
    )
)]
pub(crate) async fn get_group_library_resources(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Query(query): Query<LibraryResourcePageQuery>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .library
        .list_resources_page_in_project(&group, &query)
        .await
    {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/folders",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = CreateFolderRequest,
    responses(
        (status = 201, description = "Created folder", body = crate::contracts::LibraryFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn create_group_library_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<CreateFolderRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .create_folder_in_project(&group, &request)
        .await
    {
        Ok(folder) => (StatusCode::CREATED, Json(folder)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/texts",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = CreateTextRequest,
    responses(
        (status = 201, description = "Created text library entry", body = LibraryUploadResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn create_group_library_text(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<CreateTextRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .create_text_file_in_project(&group, &request)
        .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/groups/by-path/{group_path}/library/texts",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = UpsertLibraryTextRequest,
    responses(
        (status = 200, description = "Upserted text library entry", body = LibraryUploadResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn upsert_group_library_text(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<UpsertLibraryTextRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .upsert_text_file_in_project(&group, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/folders/{folder_id}/move",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("folder_id" = Uuid, Path, description = "Folder id")
    ),
    request_body = MoveFolderRequest,
    responses(
        (status = 200, description = "Moved folder", body = crate::contracts::LibraryFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or folder not found")
    )
)]
pub(crate) async fn move_group_library_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, folder_id)): Path<(String, Uuid)>,
    Json(request): Json<MoveFolderRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .source_folders
        .move_source_aware_folder_in_project(&group, folder_id, &request)
        .await
    {
        Ok(folder) => (StatusCode::OK, Json(folder)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/by-path/{group_path}/library/folders/{folder_id}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("folder_id" = Uuid, Path, description = "Folder id")
    ),
    responses(
        (status = 204, description = "Deleted folder"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or folder not found")
    )
)]
pub(crate) async fn delete_group_library_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, folder_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .source_folders
        .delete_source_aware_folder_in_project(&group, folder_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/upload",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Accepted uploaded files", body = LibraryUploadResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn upload_group_library_files(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    multipart: Multipart,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    let uploads = match read_library_uploads(multipart).await {
        Ok(uploads) => uploads,
        Err(response) => return response,
    };
    match state
        .app
        .library
        .upload_files_in_project(&group, uploads)
        .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/prepare-upload",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = PrepareLibraryUploadRequest,
    responses(
        (status = 200, description = "Upload requirement or reused file", body = PrepareLibraryUploadResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or folder not found")
    )
)]
pub(crate) async fn prepare_group_library_upload(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<PrepareLibraryUploadRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .prepare_upload_in_project(&group, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/library/files/{file_id}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("file_id" = Uuid, Path, description = "File id")
    ),
    responses(
        (status = 200, description = "Library file details", body = LibraryFileDetailResponse),
        (status = 404, description = "Group or file not found")
    )
)]
pub(crate) async fn get_group_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, file_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state.app.library.get_file_in_project(&group, file_id).await {
        Ok(file) => (StatusCode::OK, Json(file)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/library/files/{file_id}/jobs",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("file_id" = Uuid, Path, description = "File id"),
        LibraryFileJobPageQuery
    ),
    responses(
        (status = 200, description = "Paginated library file ingest jobs", body = LibraryFileJobPageResponse),
        (status = 400, description = "Invalid pagination parameters"),
        (status = 404, description = "Group or file not found")
    )
)]
pub(crate) async fn get_group_library_file_jobs(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, file_id)): Path<(String, Uuid)>,
    Query(query): Query<LibraryFileJobPageQuery>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .library
        .get_file_jobs_in_project(&group, file_id, query.page, query.page_size)
        .await
    {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/{file_id}/retry",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("file_id" = Uuid, Path, description = "Failed file id")
    ),
    responses(
        (status = 202, description = "Retry accepted", body = LibraryIngestJobResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group, file, or stored file not found"),
        (status = 409, description = "File is not failed")
    )
)]
pub(crate) async fn retry_group_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, file_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .retry_file_in_project(&group, file_id)
        .await
    {
        Ok(job) => (StatusCode::ACCEPTED, Json(job)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/{file_id}/move",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("file_id" = Uuid, Path, description = "File id")
    ),
    request_body = MoveFileRequest,
    responses(
        (status = 200, description = "Moved file", body = LibraryFileDetailResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or file not found")
    )
)]
pub(crate) async fn move_group_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, file_id)): Path<(String, Uuid)>,
    Json(request): Json<MoveFileRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .move_file_in_project(&group, file_id, &request)
        .await
    {
        Ok(file) => (StatusCode::OK, Json(file)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/by-path/{group_path}/library/files/{file_id}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("file_id" = Uuid, Path, description = "File id")
    ),
    responses(
        (status = 204, description = "Deleted file"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or file not found")
    )
)]
pub(crate) async fn delete_group_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, file_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .library
        .delete_file_in_project(&group, file_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/library/jobs/{job_id}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("job_id" = Uuid, Path, description = "Job id")
    ),
    responses(
        (status = 200, description = "Library ingest job", body = LibraryIngestJobResponse),
        (status = 404, description = "Group or job not found")
    )
)]
pub(crate) async fn get_group_library_job(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, job_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state.app.library.get_job_in_project(&group, job_id).await {
        Ok(job) => (StatusCode::OK, Json(job)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
