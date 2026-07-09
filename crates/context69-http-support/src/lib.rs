use anyhow::Error;
use axum::{
    Json,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use context69_contracts::ApiErrorResponse;

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct CurrentUser(pub AuthenticatedUser);

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .map(Self)
            .ok_or_else(|| json_error_response(StatusCode::UNAUTHORIZED, "missing bearer token"))
    }
}

pub fn json_error_response(status: StatusCode, error: impl Into<String>) -> Response {
    (
        status,
        Json(ApiErrorResponse {
            error: error.into(),
        }),
    )
        .into_response()
}

pub fn internal_error_response(error: Error) -> Response {
    let message = error.to_string();
    json_error_response(
        runtime_aware_status(&message).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        message,
    )
}

pub fn runtime_aware_status(message: &str) -> Option<StatusCode> {
    if message.contains("runtime is not configured")
        || message.contains("save runtime/provider settings and restart the service")
    {
        Some(StatusCode::SERVICE_UNAVAILABLE)
    } else {
        None
    }
}
