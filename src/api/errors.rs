use axum::{Json, http::StatusCode, response::IntoResponse};

use crate::contracts::ApiErrorResponse;

pub(crate) fn internal_error_response(error: anyhow::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

pub(crate) fn source_management_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("already exists") {
        StatusCode::CONFLICT
    } else if message.contains("unknown source") {
        StatusCode::NOT_FOUND
    } else if message.contains("must not be empty")
        || message.contains("cannot be changed")
        || message.contains("batch_size must")
        || message.contains("unsupported")
        || message.contains("unknown connection")
        || message.contains("failed to validate source")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(ApiErrorResponse { error: message })).into_response()
}

pub(crate) fn library_management_error_response(
    error: anyhow::Error,
) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("unknown folder")
        || message.contains("unknown file")
        || message.contains("unknown job")
        || message.contains("unknown target folder")
    {
        StatusCode::NOT_FOUND
    } else if message.contains("must not be empty")
        || message.contains("unsupported file type")
        || message.contains("cannot be moved")
        || message.contains("folder name")
        || message.contains("invalid folder_id")
        || message.contains("exceeds upload size limit")
        || message.contains("duplicate key value")
        || message.contains("docling")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(ApiErrorResponse { error: message })).into_response()
}

pub(crate) fn settings_management_error_response(
    error: anyhow::Error,
) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("must not be empty")
        || message.contains("must be greater than 0")
        || message.contains("must be one of")
        || message.contains("is required when")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(ApiErrorResponse { error: message })).into_response()
}

pub(crate) fn admin_user_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("admin access required") {
        StatusCode::FORBIDDEN
    } else if message.contains("user not found") {
        StatusCode::NOT_FOUND
    } else if message.contains("must not be empty")
        || message.contains("last administrator")
        || message.contains("user account is disabled")
    {
        StatusCode::BAD_REQUEST
    } else if message.contains("duplicate key value")
        || message.contains("already exists")
    {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(ApiErrorResponse { error: message })).into_response()
}
