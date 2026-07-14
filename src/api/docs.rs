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
    auth::{__path_login, __path_logout, __path_me},
    documents::{
        __path_batch_get_group_documents, __path_create_metadata_index,
        __path_delete_group_document_by_key, __path_delete_metadata_index,
        __path_get_group_document_by_key, __path_list_metadata_indexes,
        __path_query_group_documents, __path_retry_metadata_index, __path_update_metadata_index,
    },
    errors::internal_error_response,
    group_library::{
        __path_create_group_library_folder, __path_create_group_library_text,
        __path_delete_group_library_file, __path_delete_group_library_folder,
        __path_get_group_library_file, __path_get_group_library_job,
        __path_get_group_library_resources, __path_get_group_library_tree,
        __path_get_group_library_url_import_job, __path_import_group_library_file_url,
        __path_move_group_library_file, __path_move_group_library_folder,
        __path_prepare_group_library_upload, __path_retry_group_library_file,
        __path_retry_group_library_url_import_job, __path_upload_group_library_files,
        __path_upsert_group_library_text,
    },
    group_source_folders::{
        __path_create_group_source_folder, __path_sync_group_source_folder,
        __path_update_group_source_folder_config,
    },
    health::__path_healthz,
    library::{
        __path_create_library_folder, __path_create_library_text, __path_delete_library_file,
        __path_delete_library_folder, __path_get_library_file, __path_get_library_job,
        __path_get_library_resources, __path_get_library_tree, __path_move_library_file,
        __path_move_library_folder, __path_upload_library_files,
    },
    personal_access_tokens::{
        __path_create_personal_access_token, __path_list_personal_access_tokens,
        __path_revoke_personal_access_token,
    },
    sources::{
        __path_create_source, __path_create_source_connection, __path_delete_source,
        __path_delete_source_connection, __path_list_source_connections, __path_list_sources,
        __path_sync_source, __path_update_source, __path_update_source_connection,
    },
    translations::{
        __path_get_group_translation_settings, __path_get_translation_job,
        __path_get_translation_settings, __path_list_document_translation_jobs,
        __path_rebuild_document_translations, __path_retry_translation_job,
        __path_update_group_translation_settings, __path_update_translation_settings,
    },
};
use crate::contracts::{
    AdminUserPageQuery, AdminUserPageResponse, AdminUserResponse, ApiErrorResponse,
    AuthLoginRequest, AuthMeResponse, AuthUserResponse, BatchDocumentItem,
    BatchGetDocumentsRequest, BatchGetDocumentsResponse, CreateAdminUserRequest,
    CreateFolderRequest, CreateMetadataIndexRequest, CreatePersonalAccessTokenRequest,
    CreatePersonalAccessTokenResponse, CreateSourceFolderRequest, CreateTextRequest, DeeplPlan,
    DocumentKey, DocumentLookupQuery, DocumentQueryRequest, DocumentQueryResponse, DocumentSort,
    DocumentSortField, GroupTranslationSettingsResponse, HealthResponse, HealthStatus,
    ImportLibraryFileFromUrlRequest, LibraryFileDetailResponse, LibraryFileIngestOptions,
    LibraryFileUploadMetadata, LibraryFolderResponse, LibraryIngestJobResponse,
    LibraryResourceItem, LibraryResourceKind, LibraryResourcePageResponse, LibraryResourceSortBy,
    LibraryTreeResponse, LibraryUploadResponse, LibraryUrlImportJobResponse,
    LibraryUrlImportStatus, MetadataDataType, MetadataFilter, MetadataFilterOperator,
    MetadataIndexResponse, MetadataIndexStatus, MetadataValueKind, MoveFileRequest,
    MoveFolderRequest, PersonalAccessTokenResponse, PersonalAccessTokenScope,
    PrepareLibraryUploadRequest, PrepareLibraryUploadResponse, RebuildDocumentTranslationsRequest,
    ResetAdminUserPasswordRequest, SearchMode, SortDirection, SortOrder, SourceConfigInput,
    SourceConnectionResponse, SourceFolderResponse, SourceStatus, SyncOutcome,
    TranslationDirective, TranslationGlossaryEntry, TranslationJobResponse,
    TranslationJobsResponse, TranslationLlmApiKind, TranslationProviderInput,
    TranslationProviderKind, TranslationProviderResponse, TranslationSettingsResponse,
    TranslationStatus, UpdateAdminUserRequest, UpdateGroupTranslationSettingsRequest,
    UpdateMetadataIndexRequest, UpdateTranslationSettingsRequest, UpsertLibraryTextRequest,
    UpsertSourceConnectionRequest,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        healthz,
        login,
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
        get_library_resources,
        create_library_folder,
        create_library_text,
        move_library_folder,
        delete_library_folder,
        upload_library_files,
        get_library_file,
        move_library_file,
        delete_library_file,
        get_library_job,
        get_group_library_tree,
        get_group_library_resources,
        create_group_library_folder,
        create_group_library_text,
        upsert_group_library_text,
        move_group_library_folder,
        delete_group_library_folder,
        upload_group_library_files,
        import_group_library_file_url,
        get_group_library_url_import_job,
        retry_group_library_url_import_job,
        prepare_group_library_upload,
        get_group_library_file,
        move_group_library_file,
        retry_group_library_file,
        delete_group_library_file,
        get_group_library_job,
        create_group_source_folder,
        update_group_source_folder_config,
        sync_group_source_folder,
        query_group_documents,
        get_group_document_by_key,
        batch_get_group_documents,
        delete_group_document_by_key,
        list_metadata_indexes,
        create_metadata_index,
        update_metadata_index,
        retry_metadata_index,
        delete_metadata_index,
        get_translation_settings,
        update_translation_settings,
        get_group_translation_settings,
        update_group_translation_settings,
        list_document_translation_jobs,
        rebuild_document_translations,
        get_translation_job,
        retry_translation_job
    ),
    components(schemas(
        HealthStatus,
        HealthResponse,
        ApiErrorResponse,
        AuthLoginRequest,
        AuthMeResponse,
        AuthUserResponse,
        PersonalAccessTokenScope,
        CreatePersonalAccessTokenRequest,
        PersonalAccessTokenResponse,
        CreatePersonalAccessTokenResponse,
        AdminUserResponse,
        AdminUserPageResponse,
        AdminUserPageQuery,
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
        LibraryResourceKind,
        LibraryResourceSortBy,
        SortDirection,
        LibraryResourceItem,
        LibraryResourcePageResponse,
        LibraryIngestJobResponse,
        LibraryFileDetailResponse,
        LibraryFileUploadMetadata,
        ImportLibraryFileFromUrlRequest,
        LibraryUrlImportStatus,
        LibraryUrlImportJobResponse,
        LibraryUploadResponse,
        PrepareLibraryUploadRequest,
        PrepareLibraryUploadResponse,
        BatchGetDocumentsRequest,
        BatchGetDocumentsResponse,
        BatchDocumentItem,
        CreateMetadataIndexRequest,
        DocumentKey,
        DocumentLookupQuery,
        DocumentQueryRequest,
        DocumentQueryResponse,
        DocumentSort,
        DocumentSortField,
        MetadataDataType,
        MetadataFilter,
        MetadataFilterOperator,
        MetadataIndexResponse,
        MetadataIndexStatus,
        MetadataValueKind,
        SortOrder,
        UpdateMetadataIndexRequest,
        TranslationDirective,
        TranslationStatus,
        TranslationProviderKind,
        TranslationLlmApiKind,
        DeeplPlan,
        TranslationProviderInput,
        TranslationProviderResponse,
        TranslationSettingsResponse,
        TranslationGlossaryEntry,
        UpdateTranslationSettingsRequest,
        UpdateGroupTranslationSettingsRequest,
        GroupTranslationSettingsResponse,
        TranslationJobResponse,
        TranslationJobsResponse,
        RebuildDocumentTranslationsRequest,
        LibraryFileIngestOptions
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
            "/v1/auth/login",
            "/v1/auth/logout",
            "/v1/auth/me",
            "/v1/sources",
            "/v1/source-connections",
            "/v1/settings/docling",
            "/v1/settings/search",
            "/v1/search",
            "/v1/documents/{document_id}",
            "/v1/sources/{source_key}",
            "/v1/sources/{source_key}/sync",
            "/v1/groups",
            "/v1/groups/by-path/{group_path}",
            "/v1/groups/by-path/{group_path}/children",
            "/v1/groups/by-path/{group_path}/members",
            "/v1/groups/by-path/{group_path}/source-folders",
            "/v1/groups/by-path/{group_path}/source-folders/{folder_id}/config",
            "/v1/groups/by-path/{group_path}/source-folders/{folder_id}/sync",
            "/v1/groups/by-path/{group_path}/library/tree",
            "/v1/groups/by-path/{group_path}/library/folders",
            "/v1/groups/by-path/{group_path}/library/folders/{folder_id}/move",
            "/v1/groups/by-path/{group_path}/library/folders/{folder_id}",
            "/v1/groups/by-path/{group_path}/library/files/upload",
            "/v1/groups/by-path/{group_path}/library/files/{file_id}",
            "/v1/groups/by-path/{group_path}/library/files/{file_id}/move",
            "/v1/groups/by-path/{group_path}/library/files/{file_id}/retry",
            "/v1/groups/by-path/{group_path}/library/jobs/{job_id}",
        ] {
            assert!(paths.contains_key(path), "missing path {path}");
        }
        assert!(!paths.contains_key("/v1/auth/refresh"));

        let schemas = json
            .pointer("/components/schemas")
            .and_then(Value::as_object)
            .expect("schemas to exist");
        assert!(!schemas.contains_key("AuthTokenResponse"));

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
