use std::sync::Arc;

use crate::services::app::Context69App;

mod admin_users;
mod auth;
mod docs;
mod errors;
mod health;
mod library;
mod library_upload;
mod namespaces;
mod project_access;
mod project_library;
mod project_sources;
mod router;
mod settings;
mod sources;
mod user_directory;

#[derive(Clone)]
pub struct ApiState {
    pub app: Arc<Context69App>,
}

pub use docs::ApiDoc;
pub use router::router;

pub(crate) use admin_users::{
    create_admin_user, disable_admin_user, enable_admin_user, list_admin_users,
    reset_admin_user_password, update_admin_user,
};
pub(crate) use auth::{
    RequestAuth, auth_middleware, login, logout, me, optional_auth_middleware, refresh,
};
pub(crate) use docs::{get_document, openapi_json, search};
pub(crate) use health::healthz;
pub(crate) use library::{
    create_library_folder, create_library_text, delete_library_file, delete_library_folder,
    get_library_file, get_library_job, get_library_tree, move_library_file, move_library_folder,
    upload_library_files,
};
pub(crate) use namespaces::{
    create_group, create_project, delete_group, delete_group_member, delete_project,
    delete_project_member, get_group, get_project, list_group_members, list_groups,
    list_project_members, list_projects, move_project, update_group, update_project,
    upsert_group_member, upsert_project_member,
};
pub(crate) use project_library::{
    create_project_library_folder, create_project_library_text, delete_project_library_file,
    delete_project_library_folder, get_project_library_file, get_project_library_job,
    get_project_library_tree, move_project_library_file, move_project_library_folder,
    upload_project_library_files, upsert_project_library_text,
};
pub(crate) use project_sources::{
    create_project_source, delete_project_source, list_project_sources, sync_project_source,
    update_project_source,
};
pub(crate) use settings::{
    create_provider_account, delete_provider_account, get_docling_settings, get_runtime_settings,
    get_search_settings, list_provider_accounts, update_docling_settings, update_provider_account,
    update_runtime_settings, update_search_settings,
};
pub(crate) use sources::{
    create_source, create_source_connection, delete_source, delete_source_connection,
    list_source_connections, list_sources, sync_source, update_source, update_source_connection,
};
pub(crate) use user_directory::search_user_directory;
