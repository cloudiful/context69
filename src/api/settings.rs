use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::contracts::{
    DoclingSettingsResponse, ProviderAccountResponse, RuntimeSettingsResponse,
    SearchSettingsResponse, UpdateDoclingSettingsRequest, UpdateRuntimeSettingsRequest,
    UpdateSearchSettingsRequest, UpsertProviderAccountRequest,
};

use super::{
    ApiState,
    errors::{internal_error_response, settings_management_error_response},
};

#[utoipa::path(
    get,
    path = "/v1/settings/runtime",
    responses(
        (status = 200, description = "Current runtime settings", body = RuntimeSettingsResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn get_runtime_settings(State(state): State<ApiState>) -> impl IntoResponse {
    match state.app.settings.get_runtime_settings().await {
        Ok(settings) => (StatusCode::OK, Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/settings/runtime",
    request_body = UpdateRuntimeSettingsRequest,
    responses(
        (status = 200, description = "Saved runtime settings", body = RuntimeSettingsResponse),
        (status = 400, description = "Invalid runtime settings", body = crate::contracts::ApiErrorResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn update_runtime_settings(
    State(state): State<ApiState>,
    Json(request): Json<UpdateRuntimeSettingsRequest>,
) -> impl IntoResponse {
    match state.app.settings.update_runtime_settings(&request).await {
        Ok(settings) => (StatusCode::OK, Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/settings/provider-accounts",
    responses(
        (status = 200, description = "List provider accounts", body = [ProviderAccountResponse]),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn list_provider_accounts(State(state): State<ApiState>) -> impl IntoResponse {
    match state.app.settings.list_provider_accounts().await {
        Ok(accounts) => (StatusCode::OK, Json(accounts)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/settings/provider-accounts",
    request_body = UpsertProviderAccountRequest,
    responses(
        (status = 200, description = "Saved provider account", body = ProviderAccountResponse),
        (status = 400, description = "Invalid provider account", body = crate::contracts::ApiErrorResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn create_provider_account(
    State(state): State<ApiState>,
    Json(request): Json<UpsertProviderAccountRequest>,
) -> impl IntoResponse {
    save_provider_account(state, request).await
}

#[utoipa::path(
    put,
    path = "/v1/settings/provider-accounts",
    request_body = UpsertProviderAccountRequest,
    responses(
        (status = 200, description = "Saved provider account", body = ProviderAccountResponse),
        (status = 400, description = "Invalid provider account", body = crate::contracts::ApiErrorResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn update_provider_account(
    State(state): State<ApiState>,
    Json(request): Json<UpsertProviderAccountRequest>,
) -> impl IntoResponse {
    save_provider_account(state, request).await
}

async fn save_provider_account(
    state: ApiState,
    request: UpsertProviderAccountRequest,
) -> axum::response::Response {
    match state.app.settings.upsert_provider_account(&request).await {
        Ok(account) => (StatusCode::OK, Json(account)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/settings/provider-accounts/{account_key}",
    params(("account_key" = String, Path, description = "Provider account key")),
    responses(
        (status = 204, description = "Deleted provider account"),
        (status = 400, description = "Invalid provider account", body = crate::contracts::ApiErrorResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn delete_provider_account(
    State(state): State<ApiState>,
    Path(account_key): Path<String>,
) -> impl IntoResponse {
    match state
        .app
        .settings
        .delete_provider_account(&account_key)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/settings/docling",
    responses(
        (status = 200, description = "Current docling settings", body = DoclingSettingsResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn get_docling_settings(State(state): State<ApiState>) -> impl IntoResponse {
    match state.app.settings.get_docling_settings().await {
        Ok(settings) => (StatusCode::OK, Json(settings)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/settings/docling",
    request_body = UpdateDoclingSettingsRequest,
    responses(
        (status = 200, description = "Saved docling settings", body = DoclingSettingsResponse),
        (status = 400, description = "Invalid settings", body = crate::contracts::ApiErrorResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn update_docling_settings(
    State(state): State<ApiState>,
    Json(request): Json<UpdateDoclingSettingsRequest>,
) -> impl IntoResponse {
    match state.app.settings.update_docling_settings(&request).await {
        Ok(settings) => (StatusCode::OK, Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/settings/search",
    responses(
        (status = 200, description = "Current search settings", body = SearchSettingsResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn get_search_settings(State(state): State<ApiState>) -> impl IntoResponse {
    match state.app.settings.get_search_settings().await {
        Ok(settings) => (StatusCode::OK, Json(settings)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/settings/search",
    request_body = UpdateSearchSettingsRequest,
    responses(
        (status = 200, description = "Saved search settings", body = SearchSettingsResponse),
        (status = 400, description = "Invalid settings", body = crate::contracts::ApiErrorResponse),
        (status = 500, description = "Internal error", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn update_search_settings(
    State(state): State<ApiState>,
    Json(request): Json<UpdateSearchSettingsRequest>,
) -> impl IntoResponse {
    match state.app.settings.update_search_settings(&request).await {
        Ok(settings) => (StatusCode::OK, Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}
