use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{get, post, put},
};

use super::{
    ApiState, batch_get_group_documents, cancel_task, clear_task_history,
    create_group_library_folder, create_group_library_text, create_library_folder,
    create_library_text, create_metadata_index, delete_group_document_by_key,
    delete_group_library_file, delete_group_library_folder, delete_library_file,
    delete_library_folder, delete_metadata_index, delete_task, ensure_scope,
    get_group_document_by_key, get_group_library_file, get_group_library_resources,
    get_group_library_tree, get_group_translation_settings, get_library_file,
    get_library_resources, get_library_tree, get_task, import_group_library_file_url,
    list_document_extraction_jobs, list_document_translation_jobs, list_extraction_templates,
    list_metadata_indexes, list_task_items, list_tasks, move_group_library_file,
    move_group_library_folder, move_library_file, move_library_folder,
    prepare_group_library_upload, query_group_documents, rebuild_document_extractions,
    rebuild_document_translations, release_group_library_file_source,
    require_library_scope_middleware, require_workspace_scope_middleware, rerun_task, restore_task,
    retry_metadata_index, retry_task, stream_tasks, submit_delete_batch, submit_file_batch,
    submit_task, submit_text_batch, submit_url_batch, trash_task,
    update_group_translation_settings, update_metadata_index, upload_group_library_files,
    upload_library_files, upsert_extraction_template, upsert_group_library_text,
};

pub(super) fn task_routes(api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route("/v1/scopes/ensure", post(ensure_scope))
        .route("/v1/tasks/clear", post(clear_task_history))
        .route("/v1/tasks/stream", get(stream_tasks))
        .route("/v1/tasks", get(list_tasks).post(submit_task))
        .route("/v1/tasks/{task_id}", get(get_task).delete(delete_task))
        .route("/v1/tasks/{task_id}/items", get(list_task_items))
        .route("/v1/tasks/{task_id}/retry", post(retry_task))
        .route("/v1/tasks/{task_id}/rerun", post(rerun_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
        .route("/v1/tasks/{task_id}/trash", post(trash_task))
        .route("/v1/tasks/{task_id}/restore", post(restore_task))
        .route(
            "/v1/groups/by-path/{group_path}/batch/text",
            post(submit_text_batch),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/url",
            post(submit_url_batch),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/file",
            post(submit_file_batch),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/delete",
            post(submit_delete_batch),
        )
        .layer(from_fn_with_state(
            api_state,
            require_workspace_scope_middleware,
        ))
}

pub(super) fn document_routes(api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route(
            "/v1/groups/by-path/{group_path}/documents/query",
            post(query_group_documents),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/batch-get",
            post(batch_get_group_documents),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/by-external-id",
            get(get_group_document_by_key).delete(delete_group_document_by_key),
        )
        .route(
            "/v1/groups/by-path/{group_path}/metadata-indexes",
            get(list_metadata_indexes).post(create_metadata_index),
        )
        .route(
            "/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}",
            put(update_metadata_index).delete(delete_metadata_index),
        )
        .route(
            "/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}/retry",
            post(retry_metadata_index),
        )
        .route(
            "/v1/groups/by-path/{group_path}/translation-settings",
            get(get_group_translation_settings).put(update_group_translation_settings),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/{document_id}/translations",
            get(list_document_translation_jobs),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/{document_id}/translations/rebuild",
            post(rebuild_document_translations),
        )
        .route(
            "/v1/groups/by-path/{group_path}/extraction-templates",
            get(list_extraction_templates).put(upsert_extraction_template),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions",
            get(list_document_extraction_jobs),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions/rebuild",
            post(rebuild_document_extractions),
        )
        .layer(from_fn_with_state(
            api_state,
            require_library_scope_middleware,
        ))
}

pub(super) fn library_routes(upload_body_limit: usize, api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route("/v1/library/tree", get(get_library_tree))
        .route("/v1/library/resources", get(get_library_resources))
        .route("/v1/library/folders", post(create_library_folder))
        .route(
            "/v1/library/texts",
            post(create_library_text).layer(DefaultBodyLimit::max(upload_body_limit)),
        )
        .route(
            "/v1/library/folders/{folder_id}/move",
            post(move_library_folder),
        )
        .route(
            "/v1/library/folders/{folder_id}",
            axum::routing::delete(delete_library_folder),
        )
        .route(
            "/v1/library/files/upload",
            post(upload_library_files).layer(DefaultBodyLimit::max(upload_body_limit)),
        )
        .route(
            "/v1/library/files/{file_id}",
            get(get_library_file).delete(delete_library_file),
        )
        .route("/v1/library/files/{file_id}/move", post(move_library_file))
        .route(
            "/v1/groups/by-path/{group_path}/library/tree",
            get(get_group_library_tree),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/resources",
            get(get_group_library_resources),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/folders",
            post(create_group_library_folder),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/texts",
            post(create_group_library_text)
                .put(upsert_group_library_text)
                .layer(DefaultBodyLimit::max(upload_body_limit)),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/folders/{folder_id}/move",
            post(move_group_library_folder),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/folders/{folder_id}",
            axum::routing::delete(delete_group_library_folder),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/files/prepare-upload",
            post(prepare_group_library_upload),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/files/upload",
            post(upload_group_library_files).layer(DefaultBodyLimit::max(upload_body_limit)),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/files/import-url",
            post(import_group_library_file_url),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/files/{file_id}",
            get(get_group_library_file).delete(delete_group_library_file),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/files/{file_id}/move",
            post(move_group_library_file),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/files/{file_id}/release-source",
            post(release_group_library_file_source),
        )
        .layer(from_fn_with_state(
            api_state,
            require_library_scope_middleware,
        ))
}
