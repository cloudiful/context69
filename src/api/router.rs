use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    Router,
    http::{HeaderName, Method, header},
    middleware::from_fn_with_state,
    routing::{get, post, put},
};
use axum_login::AuthManagerLayerBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_sessions::{
    Expiry, MemoryStore, SessionManagerLayer, SessionStore,
    cookie::{Key, SameSite},
};
use tower_sessions_redis_store::{
    RedisStore,
    fred::{
        interfaces::ClientLike,
        prelude::{Builder, Config as FredConfig, Pool, ReconnectPolicy},
    },
};

use crate::services::app::Context69App;

use super::resource_routes::{document_routes, library_routes, task_routes};
use super::{
    ApiState, auth_middleware, build_api_state, cancel_active_tasks, create_admin_user,
    create_git_connection, create_group_source_folder, create_personal_access_token, create_source,
    create_source_connection, delete_git_repository_connection, delete_source,
    delete_source_connection, disable_admin_user, enable_admin_user,
    forbid_personal_access_token_middleware, get_extraction_health, get_git_connection_readiness,
    get_git_repository, get_git_repository_webhook, get_translation_settings, healthz,
    index_git_repository, list_admin_users, list_git_provider_connections, list_git_repositories,
    list_git_repository_files, list_personal_access_tokens, list_source_connections, list_sources,
    list_translation_providers, login, logout, me, openapi_json, receive_git_webhook,
    register_git_repository, require_admin_scope_middleware, require_search_scope_middleware,
    require_settings_scope_middleware, require_sources_scope_middleware,
    require_workspace_scope_middleware, reset_admin_user_password, revoke_personal_access_token,
    set_git_repository_connection, submit_vector_index_rebuild, sync_group_source_folder,
    sync_source, touch_personal_access_token_middleware, update_admin_user,
    update_group_source_folder_config, update_source, update_source_connection,
    update_translation_settings,
};
use crate::services::auth::{AUTH_SESSION_DATA_KEY, SESSION_COOKIE_NAME};

pub async fn router(app: Arc<Context69App>) -> Result<Router> {
    let upload_body_limit = app.library.max_upload_request_size_bytes();
    let api_state = build_api_state(app);
    match build_redis_session_store(&api_state).await {
        Ok(redis_store) => {
            log_session_configuration(&api_state, "valkey");
            build_router_with_session_store(api_state, upload_body_limit, redis_store)
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "auth session Valkey unavailable; using in-memory browser sessions until restart"
            );
            log_session_configuration(&api_state, "memory");
            build_router_with_session_store(api_state, upload_body_limit, MemoryStore::default())
        }
    }
}

fn log_session_configuration(api_state: &ApiState, backend: &'static str) {
    tracing::info!(
        session_backend = backend,
        session_cookie_name = SESSION_COOKIE_NAME,
        secure_cookies = api_state.app.config.auth.session_cookie_secure,
        idle_ttl_secs = api_state.app.config.auth.session_idle_ttl.as_secs(),
        "configured browser sessions"
    );
}

async fn build_redis_session_store(api_state: &ApiState) -> Result<RedisStore<Pool>> {
    let redis_pool = Builder::from_config(
        FredConfig::from_url(&api_state.app.browser_sessions.valkey_url)
            .context("failed to parse browser session Valkey URL")?,
    )
    .set_policy(ReconnectPolicy::new_exponential(0, 100, 30_000, 2))
    .build_pool(6)
    .context("failed to create auth session Valkey pool")?;
    redis_pool
        .init()
        .await
        .context("failed to connect auth session Valkey pool")?;
    Ok(RedisStore::new(redis_pool))
}

fn build_router_with_session_store<S: SessionStore + Clone + Send + Sync + 'static>(
    api_state: ApiState,
    upload_body_limit: usize,
    session_store: S,
) -> Result<Router> {
    let session_key = Key::from(&api_state.app.browser_sessions.signing_key);
    let session_layer = SessionManagerLayer::new(session_store)
        .with_name(SESSION_COOKIE_NAME)
        .with_path("/")
        .with_secure(api_state.app.config.auth.session_cookie_secure)
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(
            tower_sessions::cookie::time::Duration::seconds(
                api_state.app.config.auth.session_idle_ttl.as_secs() as i64,
            ),
        ))
        .with_signed(session_key);
    let auth_layer = AuthManagerLayerBuilder::new(api_state.app.auth.clone(), session_layer)
        .with_data_key(AUTH_SESSION_DATA_KEY)
        .build();
    let protected_v1 = protected_routes(upload_body_limit, api_state.clone())
        .layer(from_fn_with_state(api_state.clone(), auth_middleware));

    Ok(Router::new()
        .route("/openapi.json", get(openapi_json))
        .route("/healthz", get(healthz))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .merge(webhook_routes())
        .merge(protected_v1)
        .with_state(api_state)
        .layer(cors_layer())
        .layer(auth_layer))
}

/// The unauthenticated provider-facing webhook ingress.
///
/// It is mounted before the protected router so no auth middleware or scope
/// gate ever runs on it; the HMAC signature over the raw body is its
/// authentication, and the handler applies its own explicit body bound.
fn webhook_routes() -> Router<ApiState> {
    Router::new().route(
        "/v1/webhooks/{provider}/{external_hook_id}",
        post(receive_git_webhook),
    )
}

fn protected_routes(upload_body_limit: usize, api_state: ApiState) -> Router<ApiState> {
    general_protected_routes(api_state.clone())
        .merge(personal_access_token_management_routes(api_state.clone()))
        .merge(admin_routes(api_state.clone()))
        .merge(search_routes(api_state.clone()))
        .merge(workspace_routes(api_state.clone()))
        .merge(sources_routes(api_state.clone()))
        .merge(settings_routes(api_state.clone()))
        .merge(library_routes(upload_body_limit, api_state.clone()))
        .merge(document_routes(api_state.clone()))
        .merge(task_routes(api_state))
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
        .route("/v1/admin/tasks/cancel-active", post(cancel_active_tasks))
        .route("/v1/admin/extraction/health", get(get_extraction_health))
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
            "/v1/groups/by-path/{group_path}/source-folders",
            post(create_group_source_folder),
        )
        .route(
            "/v1/groups/by-path/{group_path}/source-folders/{folder_id}/config",
            put(update_group_source_folder_config),
        )
        .route(
            "/v1/groups/by-path/{group_path}/source-folders/{folder_id}/sync",
            post(sync_group_source_folder),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-connections",
            get(list_git_provider_connections),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-connections/{connection_key}",
            get(get_git_connection_readiness).put(create_git_connection),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-repositories",
            get(list_git_repositories).post(register_git_repository),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}",
            get(get_git_repository),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/index",
            post(index_git_repository),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/files",
            get(list_git_repository_files),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/webhook",
            get(get_git_repository_webhook),
        )
        .route(
            "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/connection",
            put(set_git_repository_connection).delete(delete_git_repository_connection),
        )
        .layer(from_fn_with_state(
            api_state,
            require_sources_scope_middleware,
        ))
}

fn settings_routes(api_state: ApiState) -> Router<ApiState> {
    context69_settings_http::router::<ApiState>()
        .route(
            "/v1/settings/runtime/vector-index/rebuild",
            post(submit_vector_index_rebuild),
        )
        .route(
            "/v1/settings/translation",
            get(get_translation_settings).put(update_translation_settings),
        )
        .route(
            "/v1/settings/translation/providers",
            get(list_translation_providers),
        )
        .layer(from_fn_with_state(
            api_state,
            require_settings_scope_middleware,
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
