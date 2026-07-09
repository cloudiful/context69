use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, Method, header},
    middleware::from_fn_with_state,
    routing::{get, post, put},
};
use tower_http::cors::{Any, CorsLayer};

use crate::services::app::Context69App;

use super::{
    ApiState, auth_middleware, build_api_state, create_admin_user, create_library_folder,
    create_library_text, create_personal_access_token, create_project_library_folder,
    create_project_library_text, create_project_source, create_source, create_source_connection,
    delete_library_file, delete_library_folder, delete_project_library_file,
    delete_project_library_folder, delete_project_source, delete_source, delete_source_connection,
    disable_admin_user, enable_admin_user, forbid_personal_access_token_middleware,
    get_library_file, get_library_job, get_library_tree, get_project_library_file,
    get_project_library_job, get_project_library_tree, healthz, list_admin_users,
    list_personal_access_tokens, list_project_sources, list_source_connections, list_sources,
    login, logout, me, move_library_file, move_library_folder, move_project_library_file,
    move_project_library_folder, openapi_json, refresh, require_admin_scope_middleware,
    require_library_scope_middleware, require_search_scope_middleware,
    require_settings_scope_middleware, require_sources_scope_middleware,
    require_workspace_scope_middleware, reset_admin_user_password, revoke_personal_access_token,
    sync_project_source, sync_source, touch_personal_access_token_middleware, update_admin_user,
    update_project_source, update_source, update_source_connection, upload_library_files,
    upload_project_library_files, upsert_project_library_text,
};

pub fn router(app: Arc<Context69App>) -> Router {
    let upload_body_limit = app.library.max_upload_request_size_bytes();
    let api_state = build_api_state(app);
    let protected_v1 = protected_routes(upload_body_limit, api_state.clone())
        .layer(from_fn_with_state(api_state.clone(), auth_middleware));

    Router::new()
        .route("/openapi.json", get(openapi_json))
        .route("/healthz", get(healthz))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/refresh", post(refresh))
        .route("/v1/auth/logout", post(logout))
        .merge(protected_v1)
        .with_state(api_state)
        .layer(cors_layer())
}

fn protected_routes(upload_body_limit: usize, api_state: ApiState) -> Router<ApiState> {
    general_protected_routes(api_state.clone())
        .merge(personal_access_token_management_routes(api_state.clone()))
        .merge(admin_routes(api_state.clone()))
        .merge(search_routes(api_state.clone()))
        .merge(workspace_routes(api_state.clone()))
        .merge(sources_routes(api_state.clone()))
        .merge(settings_routes(api_state.clone()))
        .merge(library_routes(upload_body_limit, api_state))
}

fn general_protected_routes(api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route("/v1/auth/me", get(me))
        .layer(from_fn_with_state(
            api_state,
            touch_personal_access_token_middleware,
        ))
}

fn personal_access_token_management_routes(api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route(
            "/v1/auth/personal-access-tokens",
            get(list_personal_access_tokens).post(create_personal_access_token),
        )
        .route(
            "/v1/auth/personal-access-tokens/{token_id}",
            axum::routing::delete(revoke_personal_access_token),
        )
        .layer(from_fn_with_state(
            api_state,
            forbid_personal_access_token_middleware,
        ))
}

fn admin_routes(api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route(
            "/v1/admin/users",
            get(list_admin_users).post(create_admin_user),
        )
        .route(
            "/v1/admin/users/{login_name}",
            axum::routing::patch(update_admin_user),
        )
        .route(
            "/v1/admin/users/{login_name}/disable",
            post(disable_admin_user),
        )
        .route(
            "/v1/admin/users/{login_name}/enable",
            post(enable_admin_user),
        )
        .route(
            "/v1/admin/users/{login_name}/reset-password",
            post(reset_admin_user_password),
        )
        .layer(from_fn_with_state(
            api_state,
            require_admin_scope_middleware,
        ))
}

fn search_routes(api_state: ApiState) -> Router<ApiState> {
    context69_search_http::router::<ApiState>().layer(from_fn_with_state(
        api_state,
        require_search_scope_middleware,
    ))
}

fn workspace_routes(api_state: ApiState) -> Router<ApiState> {
    context69_namespace_http::router::<ApiState>().layer(from_fn_with_state(
        api_state,
        require_workspace_scope_middleware,
    ))
}

fn sources_routes(api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route("/v1/sources", get(list_sources).post(create_source))
        .route(
            "/v1/source-connections",
            get(list_source_connections)
                .post(create_source_connection)
                .put(update_source_connection),
        )
        .route(
            "/v1/source-connections/{name}",
            axum::routing::delete(delete_source_connection),
        )
        .route(
            "/v1/sources/{source_key}",
            put(update_source).delete(delete_source),
        )
        .route("/v1/sources/{source_key}/sync", post(sync_source))
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/sources",
            get(list_project_sources).post(create_project_source),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}",
            put(update_project_source).delete(delete_project_source),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}/sync",
            post(sync_project_source),
        )
        .layer(from_fn_with_state(
            api_state,
            require_sources_scope_middleware,
        ))
}

fn settings_routes(api_state: ApiState) -> Router<ApiState> {
    context69_settings_http::router::<ApiState>().layer(from_fn_with_state(
        api_state,
        require_settings_scope_middleware,
    ))
}

fn library_routes(upload_body_limit: usize, api_state: ApiState) -> Router<ApiState> {
    Router::new()
        .route("/v1/library/tree", get(get_library_tree))
        .route("/v1/library/folders", post(create_library_folder))
        .route("/v1/library/texts", post(create_library_text))
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
        .route("/v1/library/jobs/{job_id}", get(get_library_job))
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/tree",
            get(get_project_library_tree),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/folders",
            post(create_project_library_folder),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/texts",
            post(create_project_library_text).put(upsert_project_library_text),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}/move",
            post(move_project_library_folder),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}",
            axum::routing::delete(delete_project_library_folder),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/files/upload",
            post(upload_project_library_files).layer(DefaultBodyLimit::max(upload_body_limit)),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}",
            get(get_project_library_file).delete(delete_project_library_file),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}/move",
            post(move_project_library_file),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/library/jobs/{job_id}",
            get(get_project_library_job),
        )
        .layer(from_fn_with_state(
            api_state,
            require_library_scope_middleware,
        ))
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-requested-with"),
        ])
}
