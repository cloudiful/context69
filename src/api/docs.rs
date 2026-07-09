use axum::{
    http::{StatusCode, header},
    response::IntoResponse,
};
use utoipa::OpenApi;

use crate::api::{
    admin_users::{
        __path_create_admin_user, __path_disable_admin_user, __path_enable_admin_user,
        __path_list_admin_users, __path_reset_admin_user_password, __path_update_admin_user,
    },
    auth::{__path_login, __path_logout, __path_me, __path_refresh},
    errors::internal_error_response,
    health::__path_healthz,
    library::{
        __path_create_library_folder, __path_create_library_text, __path_delete_library_file,
        __path_delete_library_folder, __path_get_library_file, __path_get_library_job,
        __path_get_library_tree, __path_move_library_file, __path_move_library_folder,
        __path_upload_library_files,
    },
    personal_access_tokens::{
        __path_create_personal_access_token, __path_list_personal_access_tokens,
        __path_revoke_personal_access_token,
    },
    project_library::{
        __path_create_project_library_folder, __path_create_project_library_text,
        __path_delete_project_library_file, __path_delete_project_library_folder,
        __path_get_project_library_file, __path_get_project_library_job,
        __path_get_project_library_tree, __path_move_project_library_file,
        __path_move_project_library_folder, __path_upload_project_library_files,
        __path_upsert_project_library_text,
    },
    project_source_folders::{
        __path_create_project_source_folder, __path_sync_project_source_folder,
        __path_update_project_source_folder_config,
    },
    sources::{
        __path_create_source, __path_create_source_connection, __path_delete_source,
        __path_delete_source_connection, __path_list_source_connections, __path_list_sources,
        __path_sync_source, __path_update_source, __path_update_source_connection,
    },
};
use crate::contracts::{
    AdminUserResponse, ApiErrorResponse, AuthLoginRequest, AuthMeResponse, AuthTokenResponse,
    AuthUserResponse, CreateAdminUserRequest, CreateFolderRequest,
    CreatePersonalAccessTokenRequest, CreatePersonalAccessTokenResponse, CreateSourceFolderRequest,
    CreateTextRequest, HealthResponse, HealthStatus, LibraryFileDetailResponse,
    LibraryFolderResponse, LibraryIngestJobResponse, LibraryTreeResponse, LibraryUploadResponse,
    MoveFileRequest, MoveFolderRequest, PersonalAccessTokenResponse, PersonalAccessTokenScope,
    ResetAdminUserPasswordRequest, SearchMode, SourceConfigInput, SourceConnectionResponse,
    SourceFolderResponse, SourceStatus, SyncOutcome, UpdateAdminUserRequest,
    UpsertLibraryTextRequest, UpsertSourceConnectionRequest,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        healthz,
        login,
        refresh,
        logout,
        me,
        list_personal_access_tokens,
        create_personal_access_token,
        revoke_personal_access_token,
        list_admin_users,
        create_admin_user,
        update_admin_user,
        reset_admin_user_password,
        disable_admin_user,
        enable_admin_user,
        list_sources,
        list_source_connections,
        create_source_connection,
        update_source_connection,
        delete_source_connection,
        create_source,
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
        create_project_source_folder,
        update_project_source_folder_config,
        sync_project_source_folder
    ),
    components(schemas(
        HealthStatus,
        HealthResponse,
        ApiErrorResponse,
        AuthLoginRequest,
        AuthTokenResponse,
        AuthMeResponse,
        AuthUserResponse,
        PersonalAccessTokenScope,
        CreatePersonalAccessTokenRequest,
        PersonalAccessTokenResponse,
        CreatePersonalAccessTokenResponse,
        AdminUserResponse,
        CreateAdminUserRequest,
        UpdateAdminUserRequest,
        ResetAdminUserPasswordRequest,
        SearchMode,
        SourceStatus,
        SourceConfigInput,
        SourceConnectionResponse,
        UpsertSourceConnectionRequest,
        SyncOutcome,
        CreateSourceFolderRequest,
        SourceFolderResponse,
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

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    let mut document = ApiDoc::openapi();
    document.merge(context69_namespace_http::openapi_document());
    document.merge(context69_search_http::openapi_document());
    document.merge(context69_settings_http::openapi_document());
    document
}

pub(crate) async fn openapi_json() -> impl IntoResponse {
    match tokio::task::spawn_blocking(|| serde_json::to_vec(&openapi_document())).await {
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

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::openapi_document;

    #[test]
    fn openapi_contains_expected_paths_and_schemas() {
        let json = serde_json::to_value(openapi_document()).expect("openapi to serialize");
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
            "/v1/groups",
            "/v1/groups/{group_key}",
            "/v1/groups/{group_key}/projects/{project_key}",
            "/v1/groups/{group_key}/projects/{project_key}/source-folders",
            "/v1/groups/{group_key}/projects/{project_key}/source-folders/{folder_id}/config",
            "/v1/groups/{group_key}/projects/{project_key}/source-folders/{folder_id}/sync",
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
            "GroupResponse",
            "ProjectResponse",
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
    }
}
