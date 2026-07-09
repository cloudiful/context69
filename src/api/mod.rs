use std::sync::Arc;

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
mod errors;
mod health;
mod library;
mod library_upload;
mod personal_access_tokens;
mod project_access;
mod project_library;
mod project_source_folders;
mod router;
mod sources;

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
            search: Arc::new(SearchApiAdapter::new(app.query.clone(), app.auth.clone())),
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

pub use docs::{ApiDoc, openapi_document};
pub use router::router;

pub(crate) use admin_users::{
    create_admin_user, disable_admin_user, enable_admin_user, list_admin_users,
    reset_admin_user_password, update_admin_user,
};
pub(crate) use auth::{
    RequestAuth, auth_middleware, forbid_personal_access_token_middleware, login, logout, me,
    optional_auth_middleware, refresh, require_admin_scope_middleware,
    require_library_scope_middleware, require_search_scope_middleware,
    require_settings_scope_middleware, require_sources_scope_middleware,
    require_workspace_scope_middleware, touch_personal_access_token_middleware,
};
pub(crate) use docs::openapi_json;
pub(crate) use health::healthz;
pub(crate) use library::{
    create_library_folder, create_library_text, delete_library_file, delete_library_folder,
    get_library_file, get_library_job, get_library_tree, move_library_file, move_library_folder,
    upload_library_files,
};
pub(crate) use personal_access_tokens::{
    create_personal_access_token, list_personal_access_tokens, revoke_personal_access_token,
};
pub(crate) use project_library::{
    create_project_library_folder, create_project_library_text, delete_project_library_file,
    delete_project_library_folder, get_project_library_file, get_project_library_job,
    get_project_library_tree, move_project_library_file, move_project_library_folder,
    upload_project_library_files, upsert_project_library_text,
};
pub(crate) use project_source_folders::{
    create_project_source_folder, sync_project_source_folder, update_project_source_folder_config,
};
pub(crate) use sources::{
    create_source, create_source_connection, delete_source, delete_source_connection,
    list_source_connections, list_sources, sync_source, update_source, update_source_connection,
};
