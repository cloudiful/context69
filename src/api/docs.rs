use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use utoipa::OpenApi;

use crate::contracts::{
    AdminUserResponse, ApiErrorResponse, AuthLoginRequest, AuthMeResponse, AuthTokenResponse,
    AuthUserResponse, CreateAdminUserRequest, CreateFolderRequest, CreateGroupRequest,
    CreateProjectRequest, CreateTextRequest, DoclingSettingsResponse, DocumentResponse,
    GroupKind,
    GroupMemberResponse, GroupResponse, HealthResponse, HealthStatus,
    LibraryFileDetailResponse, LibraryFolderResponse, LibraryIngestJobResponse,
    LibraryTreeResponse, LibraryUploadResponse, MembershipRole, MoveFileRequest,
    MoveFolderRequest, MoveProjectRequest, ProjectMemberResponse, ProjectResponse,
    ProviderAccountResponse, ResetAdminUserPasswordRequest, RuntimeSettingsResponse,
    SearchMode, SearchRequest, SearchResponse, SearchSettingsResponse,
    SourceConfigInput, SourceConnectionResponse, SourceStatus, SyncOutcome,
    UpdateAdminUserRequest, UpdateDoclingSettingsRequest, UpdateGroupRequest,
    UpdateProjectRequest, UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
    UpsertLibraryTextRequest,
    UpsertMembershipRequest, UpsertProviderAccountRequest, UpsertSourceConnectionRequest,
    UserDirectoryEntryResponse, Visibility,
};
use crate::api::{
    ApiState,
    admin_users::{
        __path_create_admin_user, __path_disable_admin_user, __path_enable_admin_user,
        __path_list_admin_users, __path_reset_admin_user_password, __path_update_admin_user,
    },
    auth::CurrentUser,
    auth::{__path_login, __path_logout, __path_me, __path_refresh},
    errors::internal_error_response,
    health::__path_healthz,
    library::{
        __path_create_library_folder, __path_delete_library_file,
        __path_delete_library_folder, __path_get_library_file, __path_get_library_job,
        __path_get_library_tree, __path_move_library_file, __path_move_library_folder,
        __path_create_library_text,
        __path_upload_library_files,
    },
    namespaces::{
        __path_create_group, __path_create_project, __path_delete_group,
        __path_delete_group_member, __path_delete_project, __path_delete_project_member,
        __path_get_group, __path_get_project, __path_list_group_members, __path_list_groups,
        __path_list_project_members, __path_list_projects, __path_move_project,
        __path_update_group, __path_update_project, __path_upsert_group_member,
        __path_upsert_project_member,
    },
    project_library::{
        __path_create_project_library_folder, __path_delete_project_library_file,
        __path_delete_project_library_folder, __path_get_project_library_file,
        __path_get_project_library_job, __path_get_project_library_tree,
        __path_move_project_library_file, __path_move_project_library_folder,
        __path_create_project_library_text, __path_upsert_project_library_text,
        __path_upload_project_library_files,
    },
    project_sources::{
        __path_create_project_source, __path_delete_project_source,
        __path_list_project_sources, __path_sync_project_source,
        __path_update_project_source,
    },
    settings::{
        __path_create_provider_account, __path_delete_provider_account,
        __path_get_docling_settings, __path_get_runtime_settings, __path_get_search_settings,
        __path_list_provider_accounts, __path_update_docling_settings,
        __path_update_provider_account, __path_update_runtime_settings,
        __path_update_search_settings,
    },
    sources::{
        __path_create_source, __path_create_source_connection, __path_delete_source,
        __path_delete_source_connection, __path_list_source_connections, __path_list_sources,
        __path_sync_source, __path_update_source, __path_update_source_connection,
    },
    user_directory::__path_search_user_directory,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        healthz,
        login,
        refresh,
        logout,
        me,
        list_admin_users,
        create_admin_user,
        update_admin_user,
        reset_admin_user_password,
        disable_admin_user,
        enable_admin_user,
        search_user_directory,
        list_sources,
        list_source_connections,
        create_source_connection,
        update_source_connection,
        delete_source_connection,
        get_runtime_settings,
        update_runtime_settings,
        list_provider_accounts,
        create_provider_account,
        update_provider_account,
        delete_provider_account,
        get_docling_settings,
        update_docling_settings,
        get_search_settings,
        update_search_settings,
        create_source,
        search,
        get_document,
        sync_source,
        update_source,
        delete_source,
        get_library_tree,
        create_library_folder,
        create_library_text,
        move_library_folder,
        delete_library_folder,
        upload_library_files,
        get_library_file,
        move_library_file,
        delete_library_file,
        get_library_job,
        list_project_sources,
        create_project_source,
        update_project_source,
        delete_project_source,
        sync_project_source,
        get_project_library_tree,
        create_project_library_folder,
        create_project_library_text,
        upsert_project_library_text,
        move_project_library_folder,
        delete_project_library_folder,
        upload_project_library_files,
        get_project_library_file,
        move_project_library_file,
        delete_project_library_file,
        get_project_library_job,
        list_groups,
        create_group,
        get_group,
        update_group,
        delete_group,
        list_group_members,
        upsert_group_member,
        delete_group_member,
        list_projects,
        create_project,
        get_project,
        update_project,
        delete_project,
        move_project,
        list_project_members,
        upsert_project_member,
        delete_project_member
    ),
    components(schemas(
        HealthStatus,
        HealthResponse,
        ApiErrorResponse,
        AuthLoginRequest,
        AuthTokenResponse,
        AuthMeResponse,
        AuthUserResponse,
        AdminUserResponse,
        CreateAdminUserRequest,
        UpdateAdminUserRequest,
        ResetAdminUserPasswordRequest,
        UserDirectoryEntryResponse,
        Visibility,
        MembershipRole,
        GroupKind,
        GroupResponse,
        ProjectResponse,
        GroupMemberResponse,
        ProjectMemberResponse,
        CreateGroupRequest,
        UpdateGroupRequest,
        CreateProjectRequest,
        UpdateProjectRequest,
        MoveProjectRequest,
        UpsertMembershipRequest,
        SearchRequest,
        SearchMode,
        SearchResponse,
        SearchSettingsResponse,
        DocumentResponse,
        SourceStatus,
        SourceConfigInput,
        SourceConnectionResponse,
        ProviderAccountResponse,
        UpsertProviderAccountRequest,
        RuntimeSettingsResponse,
        UpdateRuntimeSettingsRequest,
        UpsertSourceConnectionRequest,
        DoclingSettingsResponse,
        UpdateDoclingSettingsRequest,
        UpdateSearchSettingsRequest,
        SyncOutcome,
        CreateFolderRequest,
        CreateTextRequest,
        UpsertLibraryTextRequest,
        MoveFolderRequest,
        MoveFileRequest,
        LibraryFolderResponse,
        LibraryTreeResponse,
        LibraryIngestJobResponse,
        LibraryFileDetailResponse,
        LibraryUploadResponse
    ))
)]
pub struct ApiDoc;

pub(crate) async fn openapi_json() -> impl IntoResponse {
    match tokio::task::spawn_blocking(|| serde_json::to_vec(&ApiDoc::openapi())).await {
        Ok(Ok(body)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Ok(Err(error)) => internal_error_response(error.into()),
        Err(error) => internal_error_response(error.into()),
    }
}

#[utoipa::path(
    post,
    path = "/v1/search",
    request_body = SearchRequest,
    responses(
        (status = 200, description = "Search indexed documents", body = SearchResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn search(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<SearchRequest>,
) -> impl IntoResponse {
    match state.app.query.search(Some(session.user.id), request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/documents/{document_id}",
    params(("document_id" = i64, Path, description = "Document id")),
    responses(
        (status = 200, description = "Document details", body = DocumentResponse),
        (status = 404, description = "Document not found", body = ApiErrorResponse),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_document(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(document_id): Path<i64>,
) -> impl IntoResponse {
    let scope = match state
        .app
        .auth
        .access_scope(Some(session.user.id), None, None)
        .await
    {
        Ok(scope) => scope,
        Err(error) => return internal_error_response(error),
    };
    match state.app.query.get_document(document_id, &scope).await {
        Ok(document) => (StatusCode::OK, Json(document)).into_response(),
        Err(error) if error.to_string().contains("not found") => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use utoipa::OpenApi;

    use super::ApiDoc;

    #[test]
    fn openapi_contains_expected_paths_and_schemas() {
        let json = serde_json::to_value(ApiDoc::openapi()).expect("openapi to serialize");
        let paths = json
            .get("paths")
            .and_then(Value::as_object)
            .expect("paths to exist");

        for path in [
            "/healthz",
            "/v1/sources",
            "/v1/source-connections",
            "/v1/settings/docling",
            "/v1/settings/search",
            "/v1/search",
            "/v1/documents/{document_id}",
            "/v1/sources/{source_key}",
            "/v1/sources/{source_key}/sync",
            "/v1/groups/{group_key}/projects/{project_key}/sources",
            "/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}",
            "/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}/sync",
            "/v1/groups/{group_key}/projects/{project_key}/library/tree",
            "/v1/groups/{group_key}/projects/{project_key}/library/folders",
            "/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}/move",
            "/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}",
            "/v1/groups/{group_key}/projects/{project_key}/library/files/upload",
            "/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}",
            "/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}/move",
            "/v1/groups/{group_key}/projects/{project_key}/library/jobs/{job_id}",
        ] {
            assert!(paths.contains_key(path), "missing path {path}");
        }

        let schemas = json
            .pointer("/components/schemas")
            .and_then(Value::as_object)
            .expect("schemas to exist");

        for schema in [
            "HealthResponse",
            "ApiErrorResponse",
            "SourceStatus",
            "SourceConfigInput",
            "DoclingSettingsResponse",
            "UpdateDoclingSettingsRequest",
            "SearchSettingsResponse",
            "UpdateSearchSettingsRequest",
            "SearchRequest",
            "SearchResponse",
            "DocumentResponse",
            "SyncOutcome",
        ] {
            assert!(schemas.contains_key(schema), "missing schema {schema}");
        }

        let source_status = schemas
            .get("SourceStatus")
            .expect("SourceStatus schema to exist");
        let source_status_properties = source_status
            .get("properties")
            .and_then(Value::as_object)
            .expect("SourceStatus properties to exist");
        assert!(source_status_properties.contains_key("display_name"));
        assert!(source_status_properties.contains_key("description"));
        assert!(source_status_properties.contains_key("example_queries"));

        let source_input = schemas
            .get("SourceConfigInput")
            .expect("SourceConfigInput schema to exist");
        let source_input_properties = source_input
            .get("properties")
            .and_then(Value::as_object)
            .expect("SourceConfigInput properties to exist");
        assert!(source_input_properties.contains_key("display_name"));
        assert!(source_input_properties.contains_key("description"));
        assert!(source_input_properties.contains_key("example_queries"));
    }
}
