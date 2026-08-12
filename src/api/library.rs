use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use context69_contracts::TaskKind;
use serde_json::json;
use uuid::Uuid;

use crate::contracts::{
    ApiErrorResponse, CreateFolderRequest, CreateTextRequest, LibraryFileDetailResponse,
    LibraryFolderResponse, LibraryResourcePageQuery, LibraryResourcePageResponse,
    LibraryTreeResponse, MoveFileRequest, MoveFolderRequest,
};

use super::{
    ApiState,
    auth::CurrentUser,
    create_text_payload,
    errors::library_management_error_response,
    file_batch_payloads,
    group_access::{group_for_user, require_group_role},
    library_upload::read_library_uploads,
    submit_task_request,
};
use crate::services::tasks::TaskSubmission;

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
        (status = 202, description = "Text task accepted", body = crate::contracts::TaskRef),
        (status = 400, description = "Invalid text payload", body = ApiErrorResponse),
        (status = 503, description = "Library dependency unavailable", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_library_text(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<CreateTextRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, "public").await {
        Ok(group) => group,
        Err(error) => return super::group_access::group_access_error_response(error),
    };
    let payload = match create_text_payload(request) {
        Ok(payload) => payload,
        Err(error) => return library_management_error_response(error),
    };
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group.group_path),
            source_key: None,
            kind: TaskKind::TextBatch,
            payloads: vec![payload],
            input_storage_object_ids: Vec::new(),
            idempotency_key: None,
        },
    )
    .await
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
        (status = 202, description = "Folder delete task accepted", body = crate::contracts::TaskRef),
        (status = 404, description = "Folder not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_library_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(folder_id): Path<Uuid>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, "public").await {
        Ok(group) => group,
        Err(error) => return super::group_access::group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, context69_contracts::MembershipRole::Maintainer)
    {
        return super::group_access::group_access_error_response(error);
    }
    if let Err(error) = state
        .app
        .library
        .get_folder_record_in_project(&group, folder_id)
        .await
    {
        return library_management_error_response(error);
    }
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group.group_path),
            source_key: None,
            kind: TaskKind::DeleteBatch,
            payloads: vec![json!({"folder_id": folder_id})],
            input_storage_object_ids: Vec::new(),
            idempotency_key: None,
        },
    )
    .await
}

#[utoipa::path(
    post,
    path = "/v1/library/files/upload",
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 202, description = "File task accepted", body = crate::contracts::TaskRef),
        (status = 400, description = "Invalid upload", body = ApiErrorResponse),
        (status = 503, description = "Library dependency unavailable", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn upload_library_files(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    multipart: Multipart,
) -> impl IntoResponse {
    let uploads = match read_library_uploads(multipart).await {
        Ok(uploads) => uploads,
        Err(response) => return response,
    };
    let group = match group_for_user(&state, session.user.id, "public").await {
        Ok(group) => group,
        Err(error) => return super::group_access::group_access_error_response(error),
    };
    let payloads = match file_batch_payloads(uploads) {
        Ok(payloads) => payloads,
        Err(error) => return library_management_error_response(error),
    };
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group.group_path),
            source_key: None,
            kind: TaskKind::FileBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: None,
        },
    )
    .await
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
        (status = 202, description = "File delete task accepted", body = crate::contracts::TaskRef),
        (status = 404, description = "File not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn delete_library_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(file_id): Path<Uuid>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, "public").await {
        Ok(group) => group,
        Err(error) => return super::group_access::group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, context69_contracts::MembershipRole::Maintainer)
    {
        return super::group_access::group_access_error_response(error);
    }
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group.group_path),
            source_key: None,
            kind: TaskKind::DeleteBatch,
            payloads: vec![json!({"file_id": file_id})],
            input_storage_object_ids: Vec::new(),
            idempotency_key: None,
        },
    )
    .await
}
