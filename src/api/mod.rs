use std::sync::Arc;

use crate::services::auth::AuthService;
use axum::extract::FromRef;
use context69_namespace_http::NamespaceHttpState;
use context69_search_http::SearchHttpState;
use context69_settings_http::SettingsHttpState;

use crate::{
    http_adapters::{
        NamespaceApiAdapter, SearchApiAdapter, SettingsApiAdapter, UserDirectoryApiAdapter,
    },
    services::app::Context69App,
};

mod admin_users;
mod auth;
mod docs;
mod documents;
mod errors;
mod extractions;
mod group_access;
mod group_library;
mod group_source_folders;
mod health;
mod library;
mod library_upload;
mod personal_access_tokens;
mod router;
mod sources;
mod task_inputs;
mod task_maintenance;
mod tasks;
mod translations;

#[derive(Clone)]
pub struct ApiState {
    pub app: Arc<Context69App>,
    pub namespace_http: NamespaceHttpState,
    pub search_http: SearchHttpState,
    pub settings_http: SettingsHttpState,
}

pub(crate) fn build_api_state(app: Arc<Context69App>) -> ApiState {
    ApiState {
        app: app.clone(),
        namespace_http: NamespaceHttpState {
            namespace: Arc::new(NamespaceApiAdapter::new(app.namespace.clone())),
            user_directory: Arc::new(UserDirectoryApiAdapter::new(app.auth.clone())),
        },
        search_http: SearchHttpState {
            search: Arc::new(SearchApiAdapter::new(
                app.query.clone(),
                app.auth.clone(),
                app.db.clone(),
            )),
        },
        settings_http: SettingsHttpState {
            settings: Arc::new(SettingsApiAdapter::new(app.settings.clone())),
        },
    }
}

impl FromRef<ApiState> for NamespaceHttpState {
    fn from_ref(state: &ApiState) -> Self {
        state.namespace_http.clone()
    }
}

impl FromRef<ApiState> for SearchHttpState {
    fn from_ref(state: &ApiState) -> Self {
        state.search_http.clone()
    }
}

impl FromRef<ApiState> for SettingsHttpState {
    fn from_ref(state: &ApiState) -> Self {
        state.settings_http.clone()
    }
}

impl FromRef<ApiState> for AuthService {
    fn from_ref(state: &ApiState) -> Self {
        state.app.auth.clone()
    }
}

pub use docs::{ApiDoc, openapi_document};
pub use router::router;

pub(crate) use admin_users::{
    create_admin_user, disable_admin_user, enable_admin_user, list_admin_users,
    reset_admin_user_password, update_admin_user,
};
pub(crate) use auth::{
    RequestAuth, auth_middleware, forbid_personal_access_token_middleware, login, logout, me,
    optional_auth_middleware, require_admin_scope_middleware, require_library_scope_middleware,
    require_search_scope_middleware, require_settings_scope_middleware,
    require_sources_scope_middleware, require_workspace_scope_middleware,
    touch_personal_access_token_middleware,
};
pub(crate) use docs::openapi_json;
pub(crate) use documents::{
    batch_get_group_documents, create_metadata_index, delete_group_document_by_key,
    delete_metadata_index, get_group_document_by_key, list_metadata_indexes, query_group_documents,
    retry_metadata_index, update_metadata_index,
};
pub(crate) use extractions::{
    list_document_extraction_jobs, list_extraction_templates, rebuild_document_extractions,
    upsert_extraction_template,
};
pub(crate) use group_library::{
    create_group_library_folder, create_group_library_text, delete_group_library_file,
    delete_group_library_folder, get_group_library_file, get_group_library_resources,
    get_group_library_tree, import_group_library_file_url, move_group_library_file,
    move_group_library_folder, prepare_group_library_upload, upload_group_library_files,
    upsert_group_library_text,
};
pub(crate) use group_source_folders::{
    create_group_source_folder, sync_group_source_folder, update_group_source_folder_config,
};
pub(crate) use health::healthz;
pub(crate) use library::{
    create_library_folder, create_library_text, delete_library_file, delete_library_folder,
    get_library_file, get_library_resources, get_library_tree, move_library_file,
    move_library_folder, upload_library_files,
};
pub(crate) use personal_access_tokens::{
    create_personal_access_token, list_personal_access_tokens, revoke_personal_access_token,
};
pub(crate) use sources::{
    create_source, create_source_connection, delete_source, delete_source_connection,
    list_source_connections, list_sources, sync_source, update_source, update_source_connection,
};
pub(crate) use task_inputs::{create_text_payload, file_batch_payloads};
pub(crate) use task_maintenance::{
    cancel_active_tasks, get_task_maintenance, purge_tasks, update_task_maintenance,
};
pub(crate) use tasks::{
    cancel_task, ensure_scope, get_task, list_task_items, list_tasks, rerun_task, retry_task,
    submit_delete_batch, submit_file_batch, submit_task, submit_task_request, submit_text_batch,
    submit_url_batch, submit_vector_index_rebuild,
};
pub(crate) use translations::*;
