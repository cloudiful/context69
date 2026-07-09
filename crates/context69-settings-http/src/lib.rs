use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get},
};
use context69_contracts::{
    ApiErrorResponse, DoclingSettingsResponse, ProviderAccountResponse, RuntimeSettingsResponse,
    SearchSettingsResponse, UpdateDoclingSettingsRequest, UpdateRuntimeSettingsRequest,
    UpdateSearchSettingsRequest, UpsertProviderAccountRequest,
};
use context69_http_support::{internal_error_response, json_error_response, runtime_aware_status};
use utoipa::OpenApi;

#[async_trait]
pub trait SettingsApi: Send + Sync {
    async fn get_runtime_settings(&self) -> Result<RuntimeSettingsResponse>;
    async fn update_runtime_settings(
        &self,
        request: &UpdateRuntimeSettingsRequest,
    ) -> Result<RuntimeSettingsResponse>;
    async fn list_provider_accounts(&self) -> Result<Vec<ProviderAccountResponse>>;
    async fn upsert_provider_account(
        &self,
        request: &UpsertProviderAccountRequest,
    ) -> Result<ProviderAccountResponse>;
    async fn delete_provider_account(&self, account_key: &str) -> Result<()>;
    async fn get_docling_settings(&self) -> Result<DoclingSettingsResponse>;
    async fn update_docling_settings(
        &self,
        request: &UpdateDoclingSettingsRequest,
    ) -> Result<DoclingSettingsResponse>;
    async fn get_search_settings(&self) -> Result<SearchSettingsResponse>;
    async fn update_search_settings(
        &self,
        request: &UpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse>;
}

#[derive(Clone)]
pub struct SettingsHttpState {
    pub settings: Arc<dyn SettingsApi>,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    SettingsHttpState: FromRef<S>,
{
    Router::new()
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
            delete(delete_provider_account),
        )
        .route(
            "/v1/settings/docling",
            get(get_docling_settings).put(update_docling_settings),
        )
        .route(
            "/v1/settings/search",
            get(get_search_settings).put(update_search_settings),
        )
}

#[derive(OpenApi)]
#[openapi(
    paths(
        get_runtime_settings,
        update_runtime_settings,
        list_provider_accounts,
        create_provider_account,
        update_provider_account,
        delete_provider_account,
        get_docling_settings,
        update_docling_settings,
        get_search_settings,
        update_search_settings
    ),
    components(
        schemas(
            ApiErrorResponse,
            ProviderAccountResponse,
            RuntimeSettingsResponse,
            UpdateRuntimeSettingsRequest,
            DoclingSettingsResponse,
            UpdateDoclingSettingsRequest,
            SearchSettingsResponse,
            UpdateSearchSettingsRequest,
            UpsertProviderAccountRequest
        )
    ),
    tags((name = "settings", description = "Runtime settings transport"))
)]
struct SettingsApiDoc;

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    SettingsApiDoc::openapi()
}

#[utoipa::path(get, path = "/v1/settings/runtime", responses((status = 200, body = RuntimeSettingsResponse), (status = 500, body = ApiErrorResponse)))]
async fn get_runtime_settings(State(state): State<SettingsHttpState>) -> impl IntoResponse {
    match state.settings.get_runtime_settings().await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/settings/runtime", request_body = UpdateRuntimeSettingsRequest, responses((status = 200, body = RuntimeSettingsResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn update_runtime_settings(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpdateRuntimeSettingsRequest>,
) -> impl IntoResponse {
    match state.settings.update_runtime_settings(&request).await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/settings/provider-accounts", responses((status = 200, body = [ProviderAccountResponse]), (status = 500, body = ApiErrorResponse)))]
async fn list_provider_accounts(State(state): State<SettingsHttpState>) -> impl IntoResponse {
    match state.settings.list_provider_accounts().await {
        Ok(accounts) => (StatusCode::OK, axum::Json(accounts)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/settings/provider-accounts", request_body = UpsertProviderAccountRequest, responses((status = 200, body = ProviderAccountResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn create_provider_account(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpsertProviderAccountRequest>,
) -> impl IntoResponse {
    save_provider_account(state, request).await
}

#[utoipa::path(put, path = "/v1/settings/provider-accounts", request_body = UpsertProviderAccountRequest, responses((status = 200, body = ProviderAccountResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn update_provider_account(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpsertProviderAccountRequest>,
) -> impl IntoResponse {
    save_provider_account(state, request).await
}

async fn save_provider_account(
    state: SettingsHttpState,
    request: UpsertProviderAccountRequest,
) -> axum::response::Response {
    match state.settings.upsert_provider_account(&request).await {
        Ok(account) => (StatusCode::OK, axum::Json(account)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(delete, path = "/v1/settings/provider-accounts/{account_key}", params(("account_key" = String, Path)), responses((status = 204), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn delete_provider_account(
    State(state): State<SettingsHttpState>,
    Path(account_key): Path<String>,
) -> impl IntoResponse {
    match state.settings.delete_provider_account(&account_key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/settings/docling", responses((status = 200, body = DoclingSettingsResponse), (status = 500, body = ApiErrorResponse)))]
async fn get_docling_settings(State(state): State<SettingsHttpState>) -> impl IntoResponse {
    match state.settings.get_docling_settings().await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/settings/docling", request_body = UpdateDoclingSettingsRequest, responses((status = 200, body = DoclingSettingsResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn update_docling_settings(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpdateDoclingSettingsRequest>,
) -> impl IntoResponse {
    match state.settings.update_docling_settings(&request).await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/settings/search", responses((status = 200, body = SearchSettingsResponse), (status = 500, body = ApiErrorResponse)))]
async fn get_search_settings(State(state): State<SettingsHttpState>) -> impl IntoResponse {
    match state.settings.get_search_settings().await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/settings/search", request_body = UpdateSearchSettingsRequest, responses((status = 200, body = SearchSettingsResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn update_search_settings(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpdateSearchSettingsRequest>,
) -> impl IntoResponse {
    match state.settings.update_search_settings(&request).await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

fn settings_management_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if let Some(status) = runtime_aware_status(&message) {
        status
    } else if message.contains("must not be empty")
        || message.contains("must be greater than 0")
        || message.contains("must be one of")
        || message.contains("is required when")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    json_error_response(status, message)
}
