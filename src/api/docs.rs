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
    extractions::{
        __path_get_extraction_health, __path_list_document_extraction_jobs,
        __path_list_extraction_templates, __path_rebuild_document_extractions,
        __path_upsert_extraction_template,
    },
    group_library::{
        __path_create_group_library_folder, __path_create_group_library_text,
        __path_delete_group_library_file, __path_delete_group_library_folder,
        __path_get_group_library_file, __path_get_group_library_resources,
        __path_get_group_library_tree, __path_import_group_library_file_url,
        __path_move_group_library_file, __path_move_group_library_folder,
        __path_prepare_group_library_upload, __path_release_group_library_file_source,
        __path_upload_group_library_files, __path_upsert_group_library_text,
    },
    group_source_folders::{
        __path_create_group_source_folder, __path_sync_group_source_folder,
        __path_update_group_source_folder_config,
    },
    health::__path_healthz,
    library::{
        __path_create_library_folder, __path_create_library_text, __path_delete_library_file,
        __path_delete_library_folder, __path_get_library_file, __path_get_library_resources,
        __path_get_library_tree, __path_move_library_file, __path_move_library_folder,
        __path_upload_library_files,
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
    task_maintenance::{
        __path_cancel_active_tasks, __path_get_task_maintenance, __path_purge_tasks,
        __path_quarantine_stale_submitting, __path_queue_docling_recovery,
        __path_recover_docling_task, __path_update_task_maintenance,
    },
    tasks::{
        __path_cancel_task, __path_delete_task, __path_ensure_scope, __path_get_task,
        __path_list_task_items, __path_list_tasks, __path_rerun_task, __path_restore_task,
        __path_retry_task, __path_submit_delete_batch, __path_submit_file_batch,
        __path_submit_task, __path_submit_text_batch, __path_submit_url_batch,
        __path_submit_vector_index_rebuild, __path_trash_task,
    },
    translations::{
        __path_get_group_translation_settings, __path_get_translation_settings,
        __path_list_document_translation_jobs, __path_list_translation_providers,
        __path_rebuild_document_translations, __path_update_group_translation_settings,
        __path_update_translation_settings,
    },
};
use crate::contracts::{
    AdminUserPageQuery, AdminUserPageResponse, AdminUserResponse, AdminUserSortBy, ApiErrorCode,
    ApiErrorResponse, AuthLoginRequest, AuthMeResponse, AuthUserResponse, BatchDocumentItem,
    BatchGetDocumentsRequest, BatchGetDocumentsResponse, CancelActiveTasksResponse,
    CanonicalApiErrorResponse, CanonicalSearchRequest, CanonicalTaskListQuery,
    CanonicalUpdateSearchSettingsRequest, CreateAdminUserRequest, CreateFolderRequest,
    CreateMetadataIndexRequest, CreatePersonalAccessTokenRequest,
    CreatePersonalAccessTokenResponse, CreateSourceFolderRequest, CreateTextRequest,
    CursorPageQuery, DeeplPlan, DeleteBatchRequest, DocumentKey, DocumentLookupQuery,
    DocumentQueryRequest, DocumentQueryResponse, DocumentSort, DocumentSortField,
    EnsureScopeResponse, ExtractionDirective, ExtractionFailureClass, ExtractionHealthResponse,
    ExtractionJobResponse, ExtractionJobStatus, ExtractionJobsResponse, ExtractionResultResponse,
    ExtractionTemplateInput, ExtractionTemplateResponse, FileBatchItem, FileBatchRequest,
    GroupSortBy, GroupTranslationSettingsResponse, HealthResponse, HealthStatus,
    ImportLibraryFileFromUrlRequest, LibraryFileDetailResponse, LibraryFileIngestOptions,
    LibraryFileUploadMetadata, LibraryFolderResponse, LibraryIngestFailureStage,
    LibraryResourceItem, LibraryResourceKind, LibraryResourcePageResponse, LibraryResourceSortBy,
    LibraryTreeResponse, MemberPageQuery, MemberSortBy, MetadataDataType, MetadataFilter,
    MetadataFilterOperator, MetadataIndexPageQuery, MetadataIndexPageResponse,
    MetadataIndexResponse, MetadataIndexStatus, MetadataValueKind, MoveFileRequest,
    MoveFolderRequest, OffsetPageQuery, PersonalAccessTokenPageQuery,
    PersonalAccessTokenPageResponse, PersonalAccessTokenResponse, PersonalAccessTokenScope,
    PrepareLibraryUploadRequest, PrepareLibraryUploadResponse, PurgeTasksRequest,
    PurgeTasksResponse, QuarantineStaleSubmittingRequest, QuarantineStaleSubmittingResponse,
    QuarantinedExternalJob, QueueDoclingRecoveryRequest, QueueDoclingRecoveryResponse,
    QueuedDoclingTask, RebuildDocumentExtractionsRequest, RebuildDocumentTranslationsRequest,
    RecoverDoclingTaskRequest, RecoverDoclingTaskResponse, RecoveredDoclingTask, RerunTaskResponse,
    ResetAdminUserPasswordRequest, ScopeMetadataIndex, ScopeSpec, SearchMode, SecretPatch,
    SortDirection, SortOrder, SourceConfigInput, SourceConnectionResponse, SourceFolderResponse,
    SourcePageQuery, SourcePageResponse, SourceStatus, SyncOutcome, TaskItemResponse,
    TaskItemStatus, TaskItemsQuery, TaskItemsResponse, TaskKind, TaskListQuery, TaskListView,
    TaskMaintenanceOverview, TaskMaintenanceSettings, TaskMaintenanceStats, TaskPageResponse,
    TaskProgress, TaskPurgeMode, TaskRef, TaskResponse, TaskRetryResponse, TaskSortBy, TaskStatus,
    TaskSubmitRequest, TextBatchRequest, TranslationDirective, TranslationGlossaryEntry,
    TranslationJobResponse, TranslationJobsResponse, TranslationLlmApiKind,
    TranslationProviderInput, TranslationProviderKind, TranslationProviderPageQuery,
    TranslationProviderPageResponse, TranslationProviderResponse, TranslationSettingsResponse,
    TranslationStatus, UpdateAdminUserRequest, UpdateGroupTranslationSettingsRequest,
    UpdateMetadataIndexRequest, UpdateTaskMaintenanceSettingsRequest,
    UpdateTranslationSettingsRequest, UpsertLibraryTextRequest, UpsertSourceConnectionRequest,
    UrlBatchRequest,
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
        get_group_library_tree,
        get_group_library_resources,
        create_group_library_folder,
        create_group_library_text,
        upsert_group_library_text,
        move_group_library_folder,
        delete_group_library_folder,
        upload_group_library_files,
        import_group_library_file_url,
        prepare_group_library_upload,
        get_group_library_file,
        move_group_library_file,
        release_group_library_file_source,
        delete_group_library_file,
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
        list_translation_providers,
        update_translation_settings,
        get_group_translation_settings,
        update_group_translation_settings,
        list_document_translation_jobs,
        rebuild_document_translations,
        list_extraction_templates,
        upsert_extraction_template,
        list_document_extraction_jobs,
        rebuild_document_extractions,
        get_extraction_health,
        ensure_scope,
        submit_text_batch,
        submit_url_batch,
        submit_file_batch,
        submit_delete_batch,
        submit_task,
        submit_vector_index_rebuild,
        get_task,
        list_tasks,
        list_task_items,
        retry_task,
        rerun_task,
        cancel_task,
        trash_task,
        restore_task,
        delete_task,
        get_task_maintenance,
        update_task_maintenance,
        cancel_active_tasks,
        purge_tasks,
        recover_docling_task,
        queue_docling_recovery,
        quarantine_stale_submitting
    ),
    components(schemas(
        HealthStatus,
        HealthResponse,
        ApiErrorResponse,
        AuthLoginRequest,
        AuthMeResponse,
        AuthUserResponse,
        PersonalAccessTokenScope,
        PersonalAccessTokenPageQuery,
        PersonalAccessTokenPageResponse,
        CreatePersonalAccessTokenRequest,
        PersonalAccessTokenResponse,
        CreatePersonalAccessTokenResponse,
        AdminUserResponse,
        AdminUserPageResponse,
        AdminUserPageQuery,
        AdminUserSortBy,
        CreateAdminUserRequest,
        UpdateAdminUserRequest,
        ResetAdminUserPasswordRequest,
        SearchMode,
        SourceStatus,
        SourcePageQuery,
        SourcePageResponse,
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
        LibraryIngestFailureStage,
        LibraryFileDetailResponse,
        LibraryFileUploadMetadata,
        ImportLibraryFileFromUrlRequest,
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
        MetadataIndexPageQuery,
        MetadataIndexPageResponse,
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
        TranslationProviderPageQuery,
        TranslationProviderPageResponse,
        TranslationSettingsResponse,
        TranslationGlossaryEntry,
        UpdateTranslationSettingsRequest,
        UpdateGroupTranslationSettingsRequest,
        GroupTranslationSettingsResponse,
        TranslationJobResponse,
        TranslationJobsResponse,
        RebuildDocumentTranslationsRequest,
        ExtractionDirective,
        ExtractionTemplateInput,
        ExtractionTemplateResponse,
        ExtractionJobResponse,
        ExtractionJobStatus,
        ExtractionFailureClass,
        ExtractionJobsResponse,
        ExtractionResultResponse,
        ExtractionHealthResponse,
        RebuildDocumentExtractionsRequest,
        LibraryFileIngestOptions,
        ApiErrorCode,
        CanonicalApiErrorResponse,
        OffsetPageQuery,
        CursorPageQuery,
        CanonicalSearchRequest,
        CanonicalTaskListQuery,
        CanonicalUpdateSearchSettingsRequest,
        SecretPatch,
        EnsureScopeResponse,
        FileBatchItem,
        FileBatchRequest,
        TaskSubmitRequest,
        ScopeMetadataIndex,
        ScopeSpec,
        TaskItemResponse,
        TaskItemStatus,
        TaskItemsQuery,
        TaskItemsResponse,
        TaskKind,
        TaskListQuery,
        TaskListView,
        TaskPageResponse,
        TaskProgress,
        TaskRef,
        TaskResponse,
        TaskRetryResponse,
        RerunTaskResponse,
        TaskStatus,
        TaskSortBy,
        TaskMaintenanceOverview,
        TaskMaintenanceSettings,
        TaskMaintenanceStats,
        TaskPurgeMode,
        UpdateTaskMaintenanceSettingsRequest,
        CancelActiveTasksResponse,
        PurgeTasksRequest,
        PurgeTasksResponse,
        QuarantineStaleSubmittingRequest,
        QuarantineStaleSubmittingResponse,
        QuarantinedExternalJob,
        QueueDoclingRecoveryRequest,
        QueueDoclingRecoveryResponse,
        QueuedDoclingTask,
        RecoverDoclingTaskRequest,
        RecoverDoclingTaskResponse,
        RecoveredDoclingTask,
        TextBatchRequest,
        UrlBatchRequest,
        DeleteBatchRequest,
        GroupSortBy,
        MemberSortBy,
        MemberPageQuery
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
            "/v1/settings/runtime/vector-index/rebuild",
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
            "/v1/groups/by-path/{group_path}/extraction-templates",
            "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions",
            "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions/rebuild",
            "/v1/tasks",
            "/v1/tasks/{task_id}",
            "/v1/tasks/{task_id}/items",
            "/v1/tasks/{task_id}/retry",
            "/v1/tasks/{task_id}/cancel",
            "/v1/tasks/{task_id}/trash",
            "/v1/tasks/{task_id}/restore",
            "/v1/admin/tasks/maintenance",
            "/v1/admin/tasks/cancel-active",
            "/v1/admin/tasks/purge",
            "/v1/admin/tasks/{task_id}/recover",
            "/v1/admin/tasks/{task_id}/recover/queue",
            "/v1/admin/tasks/quarantine-submitting",
        ] {
            assert!(paths.contains_key(path), "missing path {path}");
        }
        for path in [
            "/v1/library/processing-jobs",
            "/v1/library/processing-jobs/retry-failed",
            "/v1/library/processing-jobs/cleanup-stuck",
            "/v1/library/jobs/{job_id}",
            "/v1/library/files/{file_id}/jobs",
            "/v1/groups/by-path/{group_path}/library/jobs/{job_id}",
            "/v1/groups/by-path/{group_path}/library/files/{file_id}/jobs",
            "/v1/groups/by-path/{group_path}/library/url-import-jobs/{job_id}",
        ] {
            assert!(
                !paths.contains_key(path),
                "removed path still present {path}"
            );
        }
        assert!(!paths.contains_key("/v1/auth/refresh"));
        let vector_rebuild = paths
            .get("/v1/settings/runtime/vector-index/rebuild")
            .and_then(Value::as_object)
            .expect("vector rebuild path to exist");
        assert!(vector_rebuild.contains_key("post"));
        assert!(!vector_rebuild.contains_key("get"));

        let task_actions = paths
            .get("/v1/tasks/{task_id}")
            .and_then(Value::as_object)
            .expect("task path to exist");
        assert!(task_actions.contains_key("get"));
        assert!(task_actions.contains_key("delete"));
        for path in ["/v1/tasks/{task_id}/trash", "/v1/tasks/{task_id}/restore"] {
            let action = paths
                .get(path)
                .and_then(Value::as_object)
                .expect("task trash action path to exist");
            assert!(action.contains_key("post"), "missing post for {path}");
        }

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
            "CanonicalUpdateSearchSettingsRequest",
            "SearchRequest",
            "SearchResponse",
            "DocumentResponse",
            "GroupResponse",
            "SyncOutcome",
            "TaskRef",
            "TaskResponse",
            "TaskItemResponse",
            "TaskMaintenanceOverview",
            "TaskPurgeMode",
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

        // Task 1 parity: inventory, OpenAPI operation ids, and shared schemas agree.
        assert_inventory_matches_openapi(&json);
    }

    const INVENTORY: &str = include_str!("../../docs/contracts/v0.16-inventory.md");

    fn inventory_http_section() -> String {
        let marker = "## HTTP operations";
        let start = INVENTORY
            .find(marker)
            .expect("inventory has ## HTTP operations");
        let rest = &INVENTORY[start..];
        let end = rest[marker.len()..]
            .find("\n## ")
            .map(|i| i + marker.len())
            .unwrap_or(rest.len());
        rest[..end].to_string()
    }

    fn inventory_shared_schemas() -> Vec<String> {
        let marker = "## Shared schemas";
        let start = INVENTORY
            .find(marker)
            .expect("inventory has ## Shared schemas");
        let rest = &INVENTORY[start..];
        let end = rest[marker.len()..]
            .find("\n## ")
            .map(|i| i + marker.len())
            .unwrap_or(rest.len());
        let section = &rest[..end];
        let mut out = Vec::new();
        for line in section.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with("- `") {
                continue;
            }
            let begin = trimmed.find('`').expect("backtick") + 1;
            let finish = trimmed[begin..].find('`').expect("closing") + begin;
            let name = trimmed[begin..finish].trim().to_string();
            if !name.is_empty() {
                out.push(name);
            }
        }
        out
    }

    fn assert_inventory_matches_openapi(json: &Value) {
        let section = inventory_http_section();
        let mut rows: Vec<(String, String, String)> = Vec::new();
        for line in section.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') {
                continue;
            }
            let raw: Vec<&str> = trimmed.split('|').collect();
            if raw.len() < 11 {
                continue;
            }
            let inner: Vec<String> = raw[1..raw.len() - 1]
                .iter()
                .map(|s| s.trim().to_string())
                .collect();
            if inner.len() != 9 || inner[0] == "operation_id" || inner[0].starts_with("---") {
                continue;
            }
            rows.push((inner[0].clone(), inner[1].clone(), inner[2].clone()));
        }
        assert!(!rows.is_empty(), "inventory HTTP rows must not be empty");
        let mut seen = std::collections::HashSet::new();
        for (oid, _, _) in &rows {
            assert!(
                seen.insert(oid.clone()),
                "duplicate operation_id in inventory: {oid}"
            );
        }
        let paths = json
            .get("paths")
            .and_then(Value::as_object)
            .expect("paths to exist");
        let mut openapi_ops: Vec<(String, String, String)> = Vec::new();
        let mut openapi_ids = std::collections::HashSet::new();
        for (path, methods) in paths {
            let methods = methods.as_object().expect("path object");
            for method in ["get", "post", "put", "patch", "delete"] {
                if let Some(op) = methods.get(method) {
                    let oid = op
                        .get("operationId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    assert!(!oid.is_empty(), "missing operationId for {method} {path}");
                    assert!(
                        openapi_ids.insert(oid.clone()),
                        "duplicate operationId in OpenAPI: {oid}"
                    );
                    openapi_ops.push((oid, method.to_uppercase(), path.clone()));
                }
            }
        }
        let inventory_map: std::collections::HashMap<_, _> = rows
            .iter()
            .map(|(oid, m, p)| (oid.clone(), (m.clone(), p.clone())))
            .collect();
        for (oid, method, path) in &openapi_ops {
            let found = inventory_map
                .get(oid)
                .unwrap_or_else(|| panic!("unclassified OpenAPI operation: {oid} {method} {path}"));
            assert_eq!(&found.0, method, "method mismatch for {oid}");
            assert_eq!(&found.1, path, "path mismatch for {oid}");
        }
        let openapi_map: std::collections::HashMap<_, _> = openapi_ops
            .iter()
            .map(|(oid, m, p)| (oid.clone(), (m.clone(), p.clone())))
            .collect();
        for (oid, method, path) in &rows {
            let found = openapi_map
                .get(oid)
                .unwrap_or_else(|| panic!("inventory row not in OpenAPI: {oid} {method} {path}"));
            assert_eq!(method, &found.0, "method mismatch for {oid}");
            assert_eq!(path, &found.1, "path mismatch for {oid}");
        }
        assert_eq!(
            rows.len(),
            openapi_ops.len(),
            "inventory and OpenAPI operation counts must match"
        );
        let schemas = json
            .pointer("/components/schemas")
            .and_then(Value::as_object)
            .expect("schemas to exist");
        for name in inventory_shared_schemas() {
            assert!(
                schemas.contains_key(&name),
                "inventory shared schema missing in OpenAPI: {name}"
            );
        }
    }

    #[test]
    fn openapi_has_no_duplicate_stream_or_source_key_schemas() {
        let json = serde_json::to_value(openapi_document()).expect("openapi to serialize");
        let schemas = json
            .pointer("/components/schemas")
            .and_then(Value::as_object)
            .expect("schemas to exist");
        for removed in ["SearchStreamQuery", "SourceKeyQuery"] {
            assert!(
                !schemas.contains_key(removed),
                "duplicate/private query schema must not appear in OpenAPI: {removed}"
            );
        }
        let raw = serde_json::to_string(&json).expect("openapi string");
        assert!(
            !raw.contains("SearchStreamQuery"),
            "OpenAPI must not reference removed SearchStreamQuery"
        );
        for canonical in [
            "CanonicalSearchRequest",
            "CanonicalTaskListQuery",
            "ApiErrorCode",
        ] {
            assert!(
                schemas.contains_key(canonical),
                "canonical v0.16 schema must be exposed: {canonical}"
            );
        }
    }

    #[test]
    fn openapi_canonical_task_list_has_no_trashed_and_requires_view() {
        let json = serde_json::to_value(openapi_document()).expect("openapi to serialize");
        let params = json
            .pointer("/paths/~1v1~1tasks/get/parameters")
            .and_then(Value::as_array)
            .expect("list_tasks params to exist");
        let names: Vec<&str> = params
            .iter()
            .filter_map(|p| p.get("name").and_then(Value::as_str))
            .collect();
        assert!(
            !names.contains(&"trashed"),
            "v0.16 list_tasks must not expose legacy trashed; got {names:?}"
        );
        assert!(
            names.contains(&"view"),
            "v0.16 list_tasks must require typed view; got {names:?}"
        );
        let page = params
            .iter()
            .find(|p| p.get("name").and_then(Value::as_str) == Some("page"))
            .expect("page param");
        let page_schema = page.get("schema").expect("page schema");
        assert_eq!(
            page_schema.get("minimum").and_then(Value::as_u64),
            Some(1),
            "page minimum must be 1"
        );
        assert_eq!(
            page_schema.get("maximum").and_then(Value::as_u64),
            Some(10_000),
            "page maximum must be 10000"
        );
        let page_size = params
            .iter()
            .find(|p| p.get("name").and_then(Value::as_str) == Some("page_size"))
            .expect("page_size param");
        let size_schema = page_size.get("schema").expect("page_size schema");
        assert_eq!(
            size_schema.get("minimum").and_then(Value::as_u64),
            Some(1),
            "page_size minimum must be 1"
        );
        assert_eq!(
            size_schema.get("maximum").and_then(Value::as_u64),
            Some(100),
            "page_size maximum must be 100"
        );
    }

    #[test]
    fn openapi_canonical_stream_has_no_page_and_bounds_limit() {
        let json = serde_json::to_value(openapi_document()).expect("openapi to serialize");
        let params = json
            .pointer("/paths/~1v1~1search~1stream/get/parameters")
            .and_then(Value::as_array)
            .expect("search_stream params to exist");
        let names: Vec<&str> = params
            .iter()
            .filter_map(|p| p.get("name").and_then(Value::as_str))
            .collect();
        assert!(
            !names.contains(&"page"),
            "v0.16 search_stream must not expose legacy page; got {names:?}"
        );
        assert!(names.contains(&"query"), "stream must keep query");
        assert!(names.contains(&"cursor"), "stream must keep cursor");
        let limit = params
            .iter()
            .find(|p| p.get("name").and_then(Value::as_str) == Some("limit"))
            .expect("limit param");
        let limit_schema = limit.get("schema").expect("limit schema");
        assert_eq!(
            limit_schema.get("minimum").and_then(Value::as_u64),
            Some(1),
            "stream limit minimum must be 1"
        );
        assert_eq!(
            limit_schema.get("maximum").and_then(Value::as_u64),
            Some(100),
            "stream limit maximum must be 100"
        );
    }
}
