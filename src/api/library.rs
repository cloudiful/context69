use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use uuid::Uuid;

use crate::contracts::{
    ApiErrorResponse, CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse,
    LibraryFolderResponse, LibraryIngestJobResponse, LibraryProcessingJobPageQuery,
    LibraryProcessingJobPageResponse, LibraryResourcePageQuery, LibraryResourcePageResponse,
    LibraryTreeResponse, LibraryUploadResponse, MoveFileRequest, MoveFolderRequest,
};

use super::{
    ApiState, auth::CurrentUser, errors::library_management_error_response,
    library_upload::read_library_uploads,
};

#[utoipa::path(
    get,
    path = "/v1/library/tree",
    responses(
        (status = 200, description = "Library directory tree", body = LibraryTreeResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_library_tree(State(state): State<ApiState>) -> impl IntoResponse {
    match state.app.library.list_tree().await {
        Ok(tree) => (StatusCode::OK, Json(tree)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/library/resources",
    params(LibraryResourcePageQuery),
    responses(
        (status = 200, description = "Paginated library resources", body = LibraryResourcePageResponse),
        (status = 400, description = "Invalid pagination parameters"),
        (status = 404, description = "Folder not found")
    )
)]
pub(crate) async fn get_library_resources(
    State(state): State<ApiState>,
    Query(query): Query<LibraryResourcePageQuery>,
) -> impl IntoResponse {
    match state.app.library.list_resources_page(&query).await {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/library/processing-jobs",
    params(LibraryProcessingJobPageQuery),
    responses(
        (status = 200, description = "Visible library processing jobs", body = LibraryProcessingJobPageResponse),
        (status = 400, description = "Invalid pagination parameters", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_library_processing_jobs(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<LibraryProcessingJobPageQuery>,
) -> impl IntoResponse {
    let scope = match state
        .app
        .auth
        .access_scope(Some(session.user.id), None)
        .await
    {
        Ok(scope) => scope,
        Err(error) => return library_management_error_response(error),
    };
    match state.app.library.list_processing_jobs(&scope, &query).await {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/library/folders",
    request_body = CreateFolderRequest,
    responses(
        (status = 201, description = "Created folder", body = LibraryFolderResponse),
        (status = 400, description = "Invalid folder request", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_library_folder(
    State(state): State<ApiState>,
    Json(request): Json<CreateFolderRequest>,
) -> impl IntoResponse {
    match state.app.library.create_folder(&request).await {
        Ok(folder) => (StatusCode::CREATED, Json(folder)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/library/texts",
    request_body = CreateTextRequest,
    responses(
        (status = 201, description = "Created text library entry", body = LibraryUploadResponse),
        (status = 400, description = "Invalid text payload", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_library_text(
    State(state): State<ApiState>,
    Json(request): Json<CreateTextRequest>,
) -> impl IntoResponse {
    match state.app.library.create_text_file(&request).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/library/folders/{folder_id}/move",
    params(("folder_id" = Uuid, Path, description = "Folder id")),
    request_body = MoveFolderRequest,
    responses(
        (status = 200, description = "Moved folder", body = LibraryFolderResponse),
        (status = 400, description = "Invalid move request", body = ApiErrorResponse),
        (status = 404, description = "Folder not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn move_library_folder(
    State(state): State<ApiState>,
    Path(folder_id): Path<Uuid>,
    Json(request): Json<MoveFolderRequest>,
) -> impl IntoResponse {
    match state.app.library.move_folder(folder_id, &request).await {
        Ok(folder) => (StatusCode::OK, Json(folder)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/library/folders/{folder_id}",
    params(("folder_id" = Uuid, Path, description = "Folder id")),
    responses(
        (status = 204, description = "Deleted folder and indexed data"),
        (status = 404, description = "Folder not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_library_folder(
    State(state): State<ApiState>,
    Path(folder_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.app.library.delete_folder(folder_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/library/files/upload",
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Accepted uploaded files", body = LibraryUploadResponse),
        (status = 400, description = "Invalid upload", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn upload_library_files(
    State(state): State<ApiState>,
    multipart: Multipart,
) -> impl IntoResponse {
    let uploads = match read_library_uploads(multipart).await {
        Ok(uploads) => uploads,
        Err(response) => return response,
    };
    match state.app.library.upload_files(uploads).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/library/files/{file_id}",
    params(("file_id" = Uuid, Path, description = "File id")),
    responses(
        (status = 200, description = "Library file details", body = LibraryFileDetailResponse),
        (status = 404, description = "File not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_library_file(
    State(state): State<ApiState>,
    Path(file_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.app.library.get_file(file_id).await {
        Ok(file) => (StatusCode::OK, Json(file)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/library/files/{file_id}/move",
    params(("file_id" = Uuid, Path, description = "File id")),
    request_body = MoveFileRequest,
    responses(
        (status = 200, description = "Moved library file", body = LibraryFileDetailResponse),
        (status = 400, description = "Invalid move request", body = ApiErrorResponse),
        (status = 404, description = "File not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn move_library_file(
    State(state): State<ApiState>,
    Path(file_id): Path<Uuid>,
    Json(request): Json<MoveFileRequest>,
) -> impl IntoResponse {
    match state.app.library.move_file(file_id, &request).await {
        Ok(file) => (StatusCode::OK, Json(file)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/library/files/{file_id}",
    params(("file_id" = Uuid, Path, description = "File id")),
    responses(
        (status = 204, description = "Deleted file and indexed data"),
        (status = 404, description = "File not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_library_file(
    State(state): State<ApiState>,
    Path(file_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.app.library.delete_file(file_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/library/jobs/{job_id}",
    params(("job_id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 200, description = "Library ingest job", body = LibraryIngestJobResponse),
        (status = 404, description = "Job not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_library_job(
    State(state): State<ApiState>,
    Path(job_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.app.library.get_job(job_id).await {
        Ok(job) => (StatusCode::OK, Json(job)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
