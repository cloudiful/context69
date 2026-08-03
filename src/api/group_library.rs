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
    CreateFolderRequest, CreateTextRequest, ImportLibraryFileFromUrlRequest,
    LibraryFileDetailResponse, LibraryResourcePageQuery, LibraryResourcePageResponse,
    LibraryTreeResponse, MembershipRole, MoveFileRequest, MoveFolderRequest,
    PrepareLibraryUploadRequest, PrepareLibraryUploadResponse, UpsertLibraryTextRequest,
};

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/import-url",
    params(("group_path" = String, Path)),
    request_body = ImportLibraryFileFromUrlRequest,
    responses(
        (status = 202, body = crate::contracts::TaskRef),
        (status = 403),
        (status = 404),
        (status = 503, description = "Library dependency unavailable", body = crate::contracts::ApiErrorResponse)
    )
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
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::UrlBatch,
            payloads: vec![match serde_json::to_value(request) {
                Ok(payload) => payload,
                Err(error) => return library_management_error_response(error.into()),
            }],
            idempotency_key: None,
        },
    )
    .await
}

use super::{
    ApiState,
    auth::CurrentUser,
    create_text_payload,
    errors::library_management_error_response,
    file_batch_payloads,
    group_access::{group_access_error_response, group_for_user, require_group_role},
    library_upload::read_library_uploads,
    submit_task_request,
};
use crate::services::tasks::TaskSubmission;

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/prepare-upload",
    params(("group_path" = String, Path)),
    request_body = PrepareLibraryUploadRequest,
    responses(
        (status = 200, body = PrepareLibraryUploadResponse),
        (status = 202, body = PrepareLibraryUploadResponse),
        (status = 403),
        (status = 404),
        (status = 409, body = crate::contracts::ApiErrorResponse)
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
    let mut prepared = match state
        .app
        .library
        .prepare_upload_in_project(&group, &request)
        .await
    {
        Ok(value) => value,
        Err(error) => return library_management_error_response(error),
    };
    if prepared.upload_required {
        return (StatusCode::OK, Json(prepared)).into_response();
    }
    let Some(file) = prepared.file.as_ref() else {
        return library_management_error_response(anyhow::anyhow!(
            "prepared upload did not return a file"
        ));
    };
    let task = match state
        .app
        .tasks
        .submit(TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::FileBatch,
            payloads: vec![json!({"file_id": file.file_id})],
            idempotency_key: None,
        })
        .await
    {
        Ok(task) => task,
        Err(error) => return library_management_error_response(error),
    };
    prepared.task = Some(task);
    (StatusCode::ACCEPTED, Json(prepared)).into_response()
}

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
        (status = 202, description = "Text task accepted", body = crate::contracts::TaskRef),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found"),
        (status = 503, description = "Library dependency unavailable", body = crate::contracts::ApiErrorResponse)
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
    let payload = match create_text_payload(request) {
        Ok(payload) => payload,
        Err(error) => return library_management_error_response(error),
    };
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::TextBatch,
            payloads: vec![payload],
            idempotency_key: None,
        },
    )
    .await
}

#[utoipa::path(
    put,
    path = "/v1/groups/by-path/{group_path}/library/texts",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = UpsertLibraryTextRequest,
    responses(
        (status = 202, description = "Text task accepted", body = crate::contracts::TaskRef),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found"),
        (status = 503, description = "Library dependency unavailable", body = crate::contracts::ApiErrorResponse)
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
    let payload = match serde_json::to_value(request) {
        Ok(payload) => payload,
        Err(error) => return library_management_error_response(error.into()),
    };
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::TextBatch,
            payloads: vec![payload],
            idempotency_key: None,
        },
    )
    .await
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
        (status = 202, description = "Folder delete task accepted", body = crate::contracts::TaskRef),
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
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::DeleteBatch,
            payloads: vec![json!({"folder_id": folder_id})],
            idempotency_key: None,
        },
    )
    .await
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/library/files/upload",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 202, description = "File task accepted", body = crate::contracts::TaskRef),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found"),
        (status = 503, description = "Library dependency unavailable", body = crate::contracts::ApiErrorResponse)
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
    let payloads = match file_batch_payloads(uploads) {
        Ok(payloads) => payloads,
        Err(error) => return library_management_error_response(error),
    };
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::FileBatch,
            payloads,
            idempotency_key: None,
        },
    )
    .await
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
        (status = 202, description = "File delete task accepted", body = crate::contracts::TaskRef),
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
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::DeleteBatch,
            payloads: vec![json!({"file_id": file_id})],
            idempotency_key: None,
        },
    )
    .await
}
