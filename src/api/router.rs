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
    ApiState, auth_middleware, create_admin_user, create_group, create_library_folder,
    create_library_text, create_project, create_project_library_folder,
    create_project_library_text, create_project_source, create_provider_account, create_source,
    create_source_connection, delete_group, delete_group_member, delete_library_file,
    delete_library_folder, delete_project, delete_project_library_file,
    delete_project_library_folder, delete_project_member, delete_project_source,
    delete_provider_account, delete_source, delete_source_connection, disable_admin_user,
    enable_admin_user, get_docling_settings, get_document, get_group, get_library_file,
    get_library_job, get_library_tree, get_project, get_project_library_file,
    get_project_library_job, get_project_library_tree, get_runtime_settings, get_search_settings,
    healthz, list_admin_users, list_group_members, list_groups, list_project_members,
    list_project_sources, list_projects, list_provider_accounts, list_source_connections,
    list_sources, login, logout, me, move_library_file, move_library_folder, move_project,
    move_project_library_file, move_project_library_folder, openapi_json, refresh,
    reset_admin_user_password, search, search_user_directory, sync_project_source, sync_source,
    update_admin_user, update_docling_settings, update_group, update_project,
    update_project_source, update_provider_account, update_runtime_settings,
    update_search_settings, update_source, update_source_connection, upload_library_files,
    upload_project_library_files, upsert_group_member, upsert_project_library_text,
    upsert_project_member,
};

pub fn router(app: Arc<Context69App>) -> Router {
    let upload_body_limit = app.library.max_upload_request_size_bytes();
    let api_state = ApiState { app: app.clone() };
    let protected_v1 = base_protected_routes(upload_body_limit)
        .merge(project_scoped_routes(upload_body_limit))
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

fn base_protected_routes(upload_body_limit: usize) -> Router<ApiState> {
    Router::new()
        .route("/v1/auth/me", get(me))
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
        .route("/v1/user-directory", get(search_user_directory))
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
            "/v1/settings/runtime",
            get(get_runtime_settings).put(update_runtime_settings),
        )
        .route(
            "/v1/settings/provider-accounts",
            get(list_provider_accounts)
                .post(create_provider_account)
                .put(update_provider_account),
        )
        .route(
            "/v1/settings/provider-accounts/{account_key}",
            axum::routing::delete(delete_provider_account),
        )
        .route(
            "/v1/settings/docling",
            get(get_docling_settings).put(update_docling_settings),
        )
        .route(
            "/v1/settings/search",
            get(get_search_settings).put(update_search_settings),
        )
        .route("/v1/search", post(search))
        .route("/v1/documents/{document_id}", get(get_document))
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
            "/v1/sources/{source_key}",
            put(update_source).delete(delete_source),
        )
        .route("/v1/sources/{source_key}/sync", post(sync_source))
        .route("/v1/groups", get(list_groups).post(create_group))
        .route(
            "/v1/groups/{group_key}",
            get(get_group).patch(update_group).delete(delete_group),
        )
        .route(
            "/v1/groups/{group_key}/members",
            get(list_group_members).post(upsert_group_member),
        )
        .route(
            "/v1/groups/{group_key}/members/{login_name}",
            axum::routing::delete(delete_group_member),
        )
        .route(
            "/v1/groups/{group_key}/projects",
            get(list_projects).post(create_project),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/move",
            post(move_project),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/members",
            get(list_project_members).post(upsert_project_member),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/members/{login_name}",
            axum::routing::delete(delete_project_member),
        )
}

fn project_scoped_routes(upload_body_limit: usize) -> Router<ApiState> {
    Router::new()
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
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::ACCEPT,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("last-event-id"),
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
        ])
        .expose_headers([
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
        ])
}

#[cfg(test)]
mod tests {
    use axum::{
        Router, body,
        http::{Method, Request, StatusCode, header},
        response::IntoResponse,
        routing::post,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::{cors_layer, openapi_json};
    use crate::api::ApiDoc;
    use utoipa::OpenApi;

    #[tokio::test]
    async fn openapi_route_returns_json_document() {
        let response = openapi_json().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body to read");
        let json: Value = serde_json::from_slice(&bytes).expect("body to be valid json");

        assert_eq!(json.get("openapi").and_then(Value::as_str), Some("3.1.0"));
        assert!(json.pointer("/paths/~1healthz").is_some());
    }

    #[tokio::test]
    async fn cors_layer_allows_mcp_preflight_headers() {
        let response = Router::new()
            .route("/mcp", post(async || StatusCode::OK))
            .layer(cors_layer())
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/mcp")
                    .header(header::ORIGIN, "https://inspector.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .header(
                        header::ACCESS_CONTROL_REQUEST_HEADERS,
                        "content-type,mcp-protocol-version,mcp-session-id",
                    )
                    .body(axum::body::Body::empty())
                    .expect("request to build"),
            )
            .await
            .expect("preflight to succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&header::HeaderValue::from_static("*"))
        );
    }

    #[test]
    fn api_doc_is_constructible() {
        let _ = ApiDoc::openapi();
    }
}
